"""Worker pool limits: time limit, memory limit, crash, part error.

Spec: docs/m1_spec.md, sections "Arbeitsprozesse" and "Abnahme M1" items 1, 5, 6.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import Any

import psutil

from conftest import LAUFENDE_ZUSTAENDE, fehler_von, lege_projekt_an, nahe, toml_mit

NORMAL = """\
    from build123d import Box
    from werft import teil

    @teil("Normal")
    def bauen(dep):
        return Box(2, 3, 4)
    """
# Normal: 2 * 3 * 4 = 24 mm3

ENDLOS = """\
    from werft import teil

    @teil("Endlos")
    def bauen(dep):
        n = 0
        while True:
            n += 1
    """

FRESSER = """\
    import time
    from werft import teil

    @teil("Fresser")
    def bauen(dep):
        # Allocates 200 x 10 MB = 2 GB step by step (about 4 s in total).
        # b"x" * n with a variable n is not constant-folded and touches every
        # page, so the RSS really grows.
        n = 10_000_000
        halde = []
        for _ in range(200):
            halde.append(b"x" * n)
            time.sleep(0.02)
        return None
    """

ABSTURZ = """\
    import os
    import signal
    from werft import teil

    @teil("Absturz")
    def bauen(dep):
        os.kill(os.getpid(), signal.SIGSEGV)
    """

# Line numbers are part of the oracle: line 6 holds the division.
TEILFEHLER = """\
from werft import teil

@teil("Kaputt")
def bauen(dep):
    x = 1
    return 1 / 0
"""
TEILFEHLER_ZEILE = 6


def _normal_baut(dienst: Any) -> None:
    erg = dienst.bauen(["Normal"], warten_s=60)
    n = erg["teile"]["Normal"]
    assert n["zustand"] in ("gebaut", "aus_cache"), n
    assert nahe(n["volumen"], 24.0)


def _pool_wieder_voll(dienst: Any, anzahl: int, timeout: float = 30) -> None:
    """After a kill the process is replaced: anzahl stays, all become free again."""
    ende = time.monotonic() + timeout
    while True:
        arb = dienst.status()["arbeiter"]
        if arb["anzahl"] == anzahl and arb["frei"] == anzahl:
            return
        assert time.monotonic() < ende, f"pool not refilled: {arb}"
        time.sleep(0.1)


def test_abnahme_1_endlosschleife_zeitgrenze_status_antwortet(tmp_path: Path, dienst_starter: Any) -> None:
    # Abnahme 1: zeit_s = 3 -> the endless part ends after 3-5 s; /status answers
    # each of 20 polls in < 100 ms meanwhile; afterwards a normal part builds.
    projekt = lege_projekt_an(
        tmp_path / "p",
        toml=toml_mit(zeit_s=3, anzahl=2),
        parts={"endlos.py": ENDLOS, "normal.py": NORMAL},
    )
    dienst = dienst_starter(projekt)

    t0 = time.perf_counter()
    env = dienst.bauen_env(["Endlos"], warten_s=0)
    assert env["ok"] is True, env
    auftrag_id = env["ergebnis"]["auftrag"]
    assert env["ergebnis"]["zustand"] == "laeuft"

    dauern = []
    for _ in range(20):
        status_env, dauer = dienst.anfrage("GET", "/status")
        assert status_env["ok"] is True
        dauern.append(dauer)
        # Spec /status: auftraege [{id, zustand}] shows the running job.
        eigene = [a for a in status_env["ergebnis"]["auftraege"] if str(a["id"]) == str(auftrag_id)]
        assert len(eigene) == 1, status_env["ergebnis"]["auftraege"]
        assert eigene[0]["zustand"] in LAUFENDE_ZUSTAENDE, eigene
        time.sleep(0.05)
    polls_ende = time.perf_counter() - t0
    # The polls must have happened while the build was still running (before
    # the 3 s limit could have struck).
    assert polls_ende < 3.0, f"polls took too long to be 'during' the build: {polls_ende:.2f} s"
    assert max(dauern) < 0.100, f"status durations (s): {[round(d, 4) for d in dauern]}"

    auftrag = dienst.warte_auftrag(auftrag_id, timeout=30, takt=0.05)
    dauer_gesamt = time.perf_counter() - t0
    assert 3.0 <= dauer_gesamt <= 5.0, f"endless part ended after {dauer_gesamt:.2f} s"
    assert fehler_von(auftrag["ergebnis"], "Endlos")["art"] == "zeitgrenze"

    _normal_baut(dienst)
    _pool_wieder_voll(dienst, 2)


def test_abnahme_5_speichergrenze(tmp_path: Path, dienst_starter: Any) -> None:
    # Abnahme 5: part allocating up to 2 GB with speicher_mb = 500 ->
    # art "speichergrenze" within 10 s.
    projekt = lege_projekt_an(
        tmp_path / "p",
        toml=toml_mit(zeit_s=30, speicher_mb=500, anzahl=2),
        parts={"fresser.py": FRESSER, "normal.py": NORMAL},
    )
    dienst = dienst_starter(projekt)
    t0 = time.perf_counter()
    env = dienst.bauen_env(["Fresser"], warten_s=0)
    assert env["ok"] is True, env
    auftrag = dienst.warte_auftrag(env["ergebnis"]["auftrag"], timeout=30, takt=0.05)
    dauer = time.perf_counter() - t0
    assert fehler_von(auftrag["ergebnis"], "Fresser")["art"] == "speichergrenze"
    assert dauer < 10.0, f"memory limit took {dauer:.2f} s"
    _normal_baut(dienst)
    _pool_wieder_voll(dienst, 2)


def test_abnahme_6_teilfehler_mit_zeile_pool_bleibt(tmp_path: Path, dienst_starter: Any) -> None:
    # Abnahme 6: 1/0 -> art "teilfehler" with file, line and message; the
    # process stays in the pool (no worker pid disappears).
    projekt = lege_projekt_an(
        tmp_path / "p",
        toml=toml_mit(anzahl=2),
        parts={"kaputt.py": TEILFEHLER, "normal.py": NORMAL},
    )
    dienst = dienst_starter(projekt)
    pids_vorher = dienst.arbeiter_pids()
    assert pids_vorher, "service has no worker processes"

    erg = dienst.bauen(["Kaputt"])
    f = fehler_von(erg, "Kaputt")
    assert f["art"] == "teilfehler", f
    # Ambiguity: the spec says "mit Datei, Zeile und Meldung" without key
    # names; the German keys datei/zeile/meldung are assumed.
    assert str(f["datei"]).endswith("kaputt.py"), f
    assert int(f["zeile"]) == TEILFEHLER_ZEILE, f
    assert "ZeroDivision" in f["meldung"] or "division by zero" in f["meldung"], f

    _normal_baut(dienst)
    pids_nachher = dienst.arbeiter_pids()
    assert pids_vorher <= pids_nachher, f"worker replaced after part error: {pids_vorher} -> {pids_nachher}"
    arb = dienst.status()["arbeiter"]
    assert arb["anzahl"] == 2


def test_absturz_wird_ersetzt(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec: a crashed process (segfault) is replaced; error art "absturz".
    projekt = lege_projekt_an(
        tmp_path / "p",
        toml=toml_mit(anzahl=2),
        parts={"absturz.py": ABSTURZ, "normal.py": NORMAL},
    )
    dienst = dienst_starter(projekt)
    erg = dienst.bauen(["Absturz"], warten_s=60)
    assert fehler_von(erg, "Absturz")["art"] == "absturz"
    _pool_wieder_voll(dienst, 2)
    _normal_baut(dienst)


def test_zeitgrenze_ersetzt_den_prozess(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec: on a limit the process is killed (SIGKILL) and replaced by a new one.
    projekt = lege_projekt_an(
        tmp_path / "p",
        toml=toml_mit(zeit_s=1, anzahl=1),
        parts={"endlos.py": ENDLOS, "normal.py": NORMAL},
    )
    dienst = dienst_starter(projekt)
    vorher = dienst.arbeiter_pids()
    erg = dienst.bauen(["Endlos"], warten_s=30)
    assert fehler_von(erg, "Endlos")["art"] == "zeitgrenze"
    _pool_wieder_voll(dienst, 1)
    nachher = dienst.arbeiter_pids()
    weg = vorher - nachher
    assert len(weg) >= 1, f"no worker was killed: {vorher} -> {nachher}"
    for pid in weg:
        assert not psutil.pid_exists(pid) or psutil.Process(pid).status() == psutil.STATUS_ZOMBIE
    _normal_baut(dienst)
