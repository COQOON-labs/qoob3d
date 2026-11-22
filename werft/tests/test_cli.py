"""CLI `werft`.

Spec: docs/m1_spec.md, section "CLI werft", Abnahme M1 item 7.
Every command prints the service's JSON envelope on stdout and exits with 0
for ok = true, otherwise 1. The CLI starts the service automatically.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import Any

from conftest import dienst_json, lege_projekt_an, nahe, toml_mit, werft_cli


def _huelle(env: dict) -> None:
    assert set(env) >= {"ok", "dauer_s", "warnungen", "ergebnis"}, env


def test_abnahme_7_bauen_exit_0_und_json(beispiel_projekt: Path, cli_projekte: list[Path]) -> None:
    # Abnahme 7: `werft bauen` in the example project -> exit 0, valid JSON.
    # Kiste = 40 * 30 * 20 = 24000; Deckel = 40 * 30 * 2 = 2400 (conftest).
    cli_projekte.append(beispiel_projekt)
    erg = werft_cli(["bauen"], cwd=beispiel_projekt)
    assert erg.code == 0, erg
    env = erg.json()
    _huelle(env)
    assert env["ok"] is True
    teile = env["ergebnis"]["teile"]
    assert set(env["ergebnis"]["gebaut"]) == {"Kiste", "Deckel"}
    assert nahe(teile["Kiste"]["volumen"], 24000.0)
    assert nahe(teile["Deckel"]["volumen"], 2400.0)
    # the CLI started the service automatically
    assert (beispiel_projekt / ".werft" / "dienst.json").is_file()


def test_cli_minus_c_und_status(beispiel_projekt: Path, cli_projekte: list[Path]) -> None:
    cli_projekte.append(beispiel_projekt)
    erg = werft_cli(["-C", str(beispiel_projekt), "status"])
    assert erg.code == 0, erg
    env = erg.json()
    _huelle(env)
    assert sorted(t["name"] for t in env["ergebnis"]["teile"]) == ["Deckel", "Kiste"]


def test_cli_bauen_einzelnes_teil_und_cache(beispiel_projekt: Path, cli_projekte: list[Path]) -> None:
    cli_projekte.append(beispiel_projekt)
    erg = werft_cli(["-C", str(beispiel_projekt), "bauen", "Kiste"])
    assert erg.code == 0, erg
    assert erg.json()["ergebnis"]["gebaut"] == ["Kiste"]
    erg = werft_cli(["-C", str(beispiel_projekt), "bauen", "Kiste", "Deckel"])
    assert erg.code == 0, erg
    e = erg.json()["ergebnis"]
    assert e["gebaut"] == ["Deckel"] and e["aus_cache"] == ["Kiste"], e


def test_cli_teilfehler_exit_1(tmp_path: Path, cli_projekte: list[Path]) -> None:
    # A failing build makes the command fail (exit 1); stdout still valid JSON.
    # Ambiguity: whether a part error sets the envelope's ok to false is not
    # stated explicitly; exit 0 for a failed build would hide errors from
    # scripts and agents, so ok = false / exit 1 is required here.
    projekt = lege_projekt_an(
        tmp_path / "p",
        parts={
            "k.py": """\
                from werft import teil

                @teil("Kaputt")
                def bauen(dep):
                    return 1 / 0
                """
        },
    )
    cli_projekte.append(projekt)
    erg = werft_cli(["-C", str(projekt), "bauen"])
    assert erg.code == 1, erg
    env = erg.json()
    _huelle(env)
    assert env["ok"] is False


def test_cli_auftrag_und_abbrechen(tmp_path: Path, cli_projekte: list[Path]) -> None:
    # `werft bauen --warten 0` returns a job id; `werft auftrag ID [--abbrechen]`.
    projekt = lege_projekt_an(
        tmp_path / "p",
        toml=toml_mit(zeit_s=60, anzahl=1),
        parts={
            "e.py": """\
                from werft import teil

                @teil("Endlos")
                def bauen(dep):
                    while True:
                        pass
                """
        },
    )
    cli_projekte.append(projekt)
    erg = werft_cli(["-C", str(projekt), "bauen", "Endlos", "--warten", "0"])
    assert erg.code == 0, erg
    auftrag_id = str(erg.json()["ergebnis"]["auftrag"])

    erg = werft_cli(["-C", str(projekt), "auftrag", auftrag_id])
    assert erg.code == 0, erg
    assert erg.json()["ergebnis"]["zustand"] in ("wartet", "laeuft")

    erg = werft_cli(["-C", str(projekt), "auftrag", auftrag_id, "--abbrechen"])
    assert erg.code == 0, erg

    ende = time.monotonic() + 10
    while True:
        erg = werft_cli(["-C", str(projekt), "auftrag", auftrag_id])
        zustand = erg.json()["ergebnis"]["zustand"]
        if zustand == "abgebrochen":
            break
        assert time.monotonic() < ende, erg
        time.sleep(0.1)


def test_cli_dienst_start_stop(beispiel_projekt: Path, cli_projekte: list[Path]) -> None:
    import psutil

    cli_projekte.append(beispiel_projekt)
    erg = werft_cli(["-C", str(beispiel_projekt), "dienst", "start"])
    assert erg.code == 0, erg
    pid = dienst_json(beispiel_projekt)["pid"]
    assert psutil.pid_exists(pid)
    erg = werft_cli(["-C", str(beispiel_projekt), "dienst", "stop"])
    assert erg.code == 0, erg
    proc: Any
    try:
        proc = psutil.Process(pid)
        proc.wait(timeout=10)
    except psutil.NoSuchProcess:
        pass
    assert not psutil.pid_exists(pid) or psutil.Process(pid).status() == psutil.STATUS_ZOMBIE
