"""Service werftd: envelope, status, jobs, cancel, lifecycle.

Spec: docs/m1_spec.md, sections "Dienst werftd" and "Arbeitsprozesse".

Most tests share one module-scoped service (runtime budget of the spec:
all M1 tests < 120 s). Each such test builds its own part names, so
assertions on "gebaut" do not depend on test order.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import Any

import psutil
import pytest

from conftest import (
    LAUFENDE_ZUSTAENDE,
    dienst_json,
    geteilter_dienst,
    lege_projekt_an,
    nahe,
    toml_mit,
    werft_cli,
)


def _quader(name: str, x: float, y: float, z: float) -> str:
    return f"""\
        from build123d import Box
        from werft import teil

        @teil({name!r})
        def bauen(dep):
            return Box({x}, {y}, {z})
        """


def _langsam(name: str, basis: str, schlaf_s: float) -> str:
    # Box(basis.x, basis.y, 1) after sleeping
    return f"""\
        import time
        from build123d import Box
        from werft import teil

        @teil({name!r}, haengt_von=[{basis!r}])
        def bauen(dep):
            time.sleep({schlaf_s})
            g = dep[{basis!r}].bounding_box().size
            return Box(g.X, g.Y, 1)
        """


ENDLOS = """\
    from werft import teil

    @teil("Endlos")
    def bauen(dep):
        while True:
            pass
    """

TEILE = {
    # fortschritt test: Basis 3*4*5 = 60, Langsam 3*4*1 = 12
    "basis.py": _quader("Basis", 3, 4, 5),
    "langsam.py": _langsam("Langsam", "Basis", 1.5),
    # envelope test
    "huelle.py": _quader("HuelleBasis", 1, 1, 1),
    "huelle_langsam.py": _langsam("HuelleLangsam", "HuelleBasis", 0.5),
    # direct result test: 2*5*7 = 70
    "direkt.py": _quader("Direkt", 2, 5, 7),
    # status-while-building test
    "schlaefer.py": _langsam("Schlaefer", "Direkt", 1.5),
    # after cancel
    "danach.py": _quader("Danach", 1, 2, 2),
    "endlos.py": ENDLOS,
}
TEILNAMEN = {"Basis", "Langsam", "HuelleBasis", "HuelleLangsam", "Direkt", "Schlaefer", "Danach", "Endlos"}


@pytest.fixture(scope="module")
def dienst(tmp_path_factory: Any) -> Any:
    # Empty werft.toml: all keys optional, defaults zeit_s = 60, anzahl = 2,
    # projekt.name = folder name ("werftprobe").
    yield from geteilter_dienst(tmp_path_factory, "werftprobe", toml="", parts=TEILE)


def _pruefe_huelle(env: dict) -> None:
    # Spec: every answer has {"ok": bool, "dauer_s": float, "warnungen": [str], "ergebnis": ...};
    # with ok = false additionally "fehler": {"art": str, "meldung": str, ...}.
    assert set(env) >= {"ok", "dauer_s", "warnungen", "ergebnis"}, env
    assert isinstance(env["ok"], bool)
    assert isinstance(env["dauer_s"], (int, float)) and not isinstance(env["dauer_s"], bool)
    assert env["dauer_s"] >= 0
    assert isinstance(env["warnungen"], list)
    assert all(isinstance(w, str) for w in env["warnungen"])
    if not env["ok"]:
        assert isinstance(env["fehler"], dict), env
        assert isinstance(env["fehler"]["art"], str) and env["fehler"]["art"]
        assert isinstance(env["fehler"]["meldung"], str)


def _warte_pool_voll(dienst: Any, anzahl: int = 2, timeout: float = 30) -> None:
    ende = time.monotonic() + timeout
    while True:
        arb = dienst.status()["arbeiter"]
        if arb["anzahl"] == anzahl and arb["frei"] == anzahl:
            return
        assert time.monotonic() < ende, arb
        time.sleep(0.1)


# ---- envelope ---------------------------------------------------------------


def test_huelle_bei_allen_endpunkten(dienst: Any) -> None:
    _pruefe_huelle(dienst.get("/status"))
    env = dienst.bauen_env(["HuelleBasis"], warten_s=60)
    _pruefe_huelle(env)
    assert env["ok"] is True
    env = dienst.bauen_env(["HuelleLangsam"], warten_s=0)
    _pruefe_huelle(env)
    auftrag_id = env["ergebnis"]["auftrag"]
    _pruefe_huelle(dienst.get(f"/auftrag/{auftrag_id}"))
    dienst.warte_auftrag(auftrag_id)
    _pruefe_huelle(dienst.get(f"/auftrag/{auftrag_id}"))
    _pruefe_huelle(dienst.post("/export", {"teile": ["HuelleBasis"], "format": "stl"}))


def test_huelle_bei_fehler_unbekanntes_teil(dienst: Any) -> None:
    # Building a part that does not exist is a request error: ok = false with fehler.
    env = dienst.bauen_env(["Gibtsnicht"], warten_s=10)
    _pruefe_huelle(env)
    assert env["ok"] is False
    assert "Gibtsnicht" in env["fehler"]["meldung"]


# ---- status -------------------------------------------------------------------


def test_status_aufbau(dienst: Any) -> None:
    # Spec: ergebnis {"projekt", "teile": [{"name", "datei", "zustand"}],
    # "auftraege": [{"id", "zustand"}], "arbeiter": {"anzahl", "frei"}}.
    st = dienst.status()
    assert "projekt" in st
    assert {t["name"] for t in st["teile"]} == TEILNAMEN
    for t in st["teile"]:
        assert set(t) >= {"name", "datei", "zustand"}
        assert t["zustand"] in ("aktuell", "veraltet", "fehler", "nie_gebaut")
    assert isinstance(st["auftraege"], list)
    for a in st["auftraege"]:
        assert set(a) >= {"id", "zustand"}
    # default arbeiter.anzahl = 2
    assert st["arbeiter"]["anzahl"] == 2
    assert 0 <= st["arbeiter"]["frei"] <= 2


def test_status_projektname_vorgabe_ist_ordnername(dienst: Any) -> None:
    # Spec werft.toml: all keys optional, name defaults to the folder name.
    projekt_info = dienst.status()["projekt"]
    name = projekt_info["name"] if isinstance(projekt_info, dict) else projekt_info
    assert name == "werftprobe"


def test_status_schnell_waehrend_bau(dienst: Any) -> None:
    # Spec: /status computes no geometry, < 100 ms also while jobs are running.
    env = dienst.bauen_env(["Schlaefer"], warten_s=0)
    auftrag_id = env["ergebnis"]["auftrag"]
    for _ in range(10):
        _, dauer = dienst.anfrage("GET", "/status")
        assert dauer < 0.100, f"status took {dauer * 1000:.1f} ms"
        time.sleep(0.05)
    dienst.warte_auftrag(auftrag_id)


# ---- jobs -----------------------------------------------------------------------


def test_bauen_warten_null_liefert_auftrag_und_fortschritt(dienst: Any) -> None:
    # Spec POST /bauen: not finished within warten_s -> {"auftrag": id, "zustand": "laeuft"};
    # GET /auftrag/<id> -> {"id", "zustand", "fortschritt": {"fertig", "gesamt"}, "ergebnis"}.
    # Langsam needs Basis (neither built before), so the job comprises 2 parts.
    env = dienst.bauen_env(["Langsam"], warten_s=0)
    assert env["ok"] is True, env
    assert env["ergebnis"]["zustand"] == "laeuft"
    auftrag_id = env["ergebnis"]["auftrag"]

    gesehen_laufend = False
    letzter_fertig = -1
    ende = time.monotonic() + 60
    while True:
        a = dienst.auftrag(auftrag_id)
        assert str(a["id"]) == str(auftrag_id)
        assert a["fortschritt"]["gesamt"] == 2, a
        fertig = a["fortschritt"]["fertig"]
        assert 0 <= fertig <= 2
        assert fertig >= letzter_fertig, "progress must not go backwards"
        letzter_fertig = fertig
        if a["zustand"] in LAUFENDE_ZUSTAENDE:
            gesehen_laufend = True
        else:
            break
        assert time.monotonic() < ende, a
        time.sleep(0.05)
    assert gesehen_laufend
    assert a["zustand"] == "fertig", a
    assert a["fortschritt"]["fertig"] == 2
    erg = a["ergebnis"]
    assert set(erg["gebaut"]) == {"Basis", "Langsam"}
    assert nahe(erg["teile"]["Langsam"]["volumen"], 12.0)
    assert nahe(erg["teile"]["Basis"]["volumen"], 60.0)


def test_bauen_innerhalb_warten_liefert_ergebnis_direkt(dienst: Any) -> None:
    # Spec: finished within warten_s -> ergebnis is the build result directly.
    env = dienst.bauen_env(["Direkt"], warten_s=60)
    assert env["ok"] is True
    erg = env["ergebnis"]
    assert erg["teile"]["Direkt"]["zustand"] in ("gebaut", "aus_cache")
    assert nahe(erg["teile"]["Direkt"]["volumen"], 70.0)
    # second time: from the cache, directly
    env = dienst.bauen_env(["Direkt"], warten_s=60)
    erg = env["ergebnis"]
    assert erg["gebaut"] == [] and erg["aus_cache"] == ["Direkt"], erg
    assert erg["teile"]["Direkt"]["zustand"] == "aus_cache"


def test_abbrechen(dienst: Any) -> None:
    # Spec: POST /auftrag/<id>/abbrechen cancels; running parts: kill and replace process.
    _warte_pool_voll(dienst)
    pids_vorher = dienst.arbeiter_pids()
    env = dienst.bauen_env(["Endlos"], warten_s=0)
    auftrag_id = env["ergebnis"]["auftrag"]
    time.sleep(0.5)
    env = dienst.post(f"/auftrag/{auftrag_id}/abbrechen")
    assert env["ok"] is True, env

    ende = time.monotonic() + 5
    while True:
        a = dienst.auftrag(auftrag_id)
        if a["zustand"] == "abgebrochen":
            break
        assert a["zustand"] in LAUFENDE_ZUSTAENDE, a
        assert time.monotonic() < ende, f"job not cancelled within 5 s: {a}"
        time.sleep(0.05)

    # The pool is refilled to 2 and builds again; the busy process was replaced.
    _warte_pool_voll(dienst)
    assert pids_vorher - dienst.arbeiter_pids(), "no worker process was killed"
    erg = dienst.bauen(["Danach"])
    assert erg["teile"]["Danach"]["zustand"] == "gebaut"
    assert nahe(erg["teile"]["Danach"]["volumen"], 4.0)


# ---- lifecycle ---------------------------------------------------------------


def test_dienst_json_und_localhost(dienst: Any) -> None:
    # Spec: listens on 127.0.0.1, free port; writes .werft/dienst.json {"port", "pid"}.
    daten = dienst_json(dienst.projekt)
    assert isinstance(daten["port"], int) and 0 < daten["port"] < 65536
    assert daten["pid"] == dienst.pid
    assert psutil.pid_exists(daten["pid"])
    lauschend = [
        c
        for c in psutil.Process(dienst.pid).net_connections(kind="tcp")
        if c.status == psutil.CONN_LISTEN and c.laddr.port == daten["port"]
    ]
    assert lauschend, "service does not listen on the port from dienst.json"
    assert all(c.laddr.ip == "127.0.0.1" for c in lauschend), lauschend


def test_ein_dienst_je_projekt(dienst: Any) -> None:
    # Spec: one service per project folder; a second start keeps the first one.
    werft_cli(["-C", str(dienst.projekt), "dienst", "start"])
    assert dienst_json(dienst.projekt)["pid"] == dienst.pid
    assert psutil.Process(dienst.pid).is_running()
    # a CLI command also uses the running service
    erg = werft_cli(["-C", str(dienst.projekt), "status"])
    assert erg.code == 0, erg
    assert dienst_json(dienst.projekt)["pid"] == dienst.pid


def test_dienst_importiert_build123d_nie(dienst: Any) -> None:
    # Spec: the service process never imports build123d; workers do (warm).
    # Oracle: build123d loads OCP (OpenCascade) shared libraries; they appear in
    # the memory maps of a process that imported it. The positive check on the
    # workers shows the oracle can see the import at all.
    dienst.bauen(["Direkt"])
    _warte_pool_voll(dienst)

    def hat_ocp(p: psutil.Process) -> bool:
        return any("OCP" in m.path or "build123d" in m.path for m in p.memory_maps())

    dienst_proc = psutil.Process(dienst.pid)
    assert not hat_ocp(dienst_proc), "service process has build123d/OCP loaded"
    arbeiter = [p for p in dienst_proc.children(recursive=True) if hat_ocp(p)]
    assert len(arbeiter) >= 2, "warm workers should have build123d/OCP loaded"


# ---- tests with their own service ------------------------------------------------


def test_zwei_projekte_zwei_dienste(tmp_path: Path, dienst_starter: Any) -> None:
    p1 = lege_projekt_an(tmp_path / "eins", toml=toml_mit(anzahl=1), parts={"q.py": _quader("Q", 1, 1, 1)})
    p2 = lege_projekt_an(tmp_path / "zwei", toml=toml_mit(anzahl=1), parts={"q.py": _quader("Q", 1, 1, 1)})
    d1 = dienst_starter(p1)
    d2 = dienst_starter(p2)
    assert d1.pid != d2.pid
    assert d1.port != d2.port


def test_stop_beendet_dienst_und_arbeiter(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec: `werft dienst stop` ends the service together with its workers.
    projekt = lege_projekt_an(tmp_path / "p", toml=toml_mit(anzahl=2), parts={"q.py": _quader("Q", 1, 1, 1)})
    dienst = dienst_starter(projekt)
    prozesse = [psutil.Process(dienst.pid), *psutil.Process(dienst.pid).children(recursive=True)]
    assert len(prozesse) >= 3, "expected service plus 2 workers"
    erg = werft_cli(["-C", str(projekt), "dienst", "stop"])
    assert erg.code == 0, erg
    _, lebend = psutil.wait_procs(prozesse, timeout=10)
    lebend = [p for p in lebend if p.is_running() and p.status() != psutil.STATUS_ZOMBIE]
    assert not lebend, f"still running after stop: {lebend}"
