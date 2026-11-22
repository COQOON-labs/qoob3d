"""@teil registration and return value checks.

Spec: docs/m1_spec.md, section "Teile (parts/*.py)".

Load errors are observed through the CLI (`werft bauen`), because the spec
only says "Fehler beim Laden" without naming a public loader: the CLI prints
the JSON envelope and must exit with 1 when ok is false.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from conftest import fehler_von, lege_projekt_an, nahe, werft_cli


def _bau_ueber_cli(projekt: Path, cli_projekte: list[Path]) -> tuple[int, dict]:
    cli_projekte.append(projekt)
    erg = werft_cli(["-C", str(projekt), "bauen"])
    return erg.code, erg.json()


def _ist_fehler(code: int, env: dict) -> str:
    """The load error must make the command fail; return the text to inspect."""
    assert code == 1, env
    assert env["ok"] is False, env
    assert isinstance(env["fehler"]["art"], str) and env["fehler"]["art"], env
    assert isinstance(env["fehler"]["meldung"], str), env
    return json.dumps(env["fehler"], ensure_ascii=False)


def test_doppelter_teilname_nennt_beide_dateien(tmp_path: Path, cli_projekte: list[Path]) -> None:
    # Spec: part names are unique project-wide, otherwise a load error naming both files.
    quelltext = """\
        from build123d import Box
        from werft import teil

        @teil("Zwilling")
        def bauen(dep):
            return Box(1, 1, 1)
        """
    projekt = lege_projekt_an(
        tmp_path / "p", parts={"links_teil.py": quelltext, "rechts_teil.py": quelltext}
    )
    text = _ist_fehler(*_bau_ueber_cli(projekt, cli_projekte))
    assert "Zwilling" in text
    assert "links_teil.py" in text
    assert "rechts_teil.py" in text


def test_unbekannte_abhaengigkeit_nennt_teile(tmp_path: Path, cli_projekte: list[Path]) -> None:
    # Spec: unknown name in haengt_von -> load error naming the parts.
    projekt = lege_projekt_an(
        tmp_path / "p",
        parts={
            "alpha.py": """\
                from build123d import Box
                from werft import teil

                @teil("Alpha", haengt_von=["Gibtsnicht"])
                def bauen(dep):
                    return Box(1, 1, 1)
                """
        },
    )
    text = _ist_fehler(*_bau_ueber_cli(projekt, cli_projekte))
    assert "Alpha" in text
    assert "Gibtsnicht" in text


def test_zyklus_nennt_teile(tmp_path: Path, cli_projekte: list[Path]) -> None:
    # Spec: a cycle in haengt_von -> load error naming the parts.
    # Cycle Alpha -> Beta -> Gamma -> Alpha; Delta is independent.
    def teil(name: str, dep: str | None) -> str:
        hv = f", haengt_von=[{dep!r}]" if dep else ""
        return f"""\
            from build123d import Box
            from werft import teil

            @teil({name!r}{hv})
            def bauen(dep):
                return Box(1, 1, 1)
            """

    projekt = lege_projekt_an(
        tmp_path / "p",
        parts={
            "alpha.py": teil("Alpha", "Beta"),
            "beta.py": teil("Beta", "Gamma"),
            "gamma.py": teil("Gamma", "Alpha"),
            "delta.py": teil("Delta", None),
        },
    )
    text = _ist_fehler(*_bau_ueber_cli(projekt, cli_projekte))
    for name in ("Alpha", "Beta", "Gamma"):
        assert name in text, text


def test_selbstabhaengigkeit_ist_zyklus(tmp_path: Path, cli_projekte: list[Path]) -> None:
    projekt = lege_projekt_an(
        tmp_path / "p",
        parts={
            "ich.py": """\
                from build123d import Box
                from werft import teil

                @teil("Narziss", haengt_von=["Narziss"])
                def bauen(dep):
                    return Box(1, 1, 1)
                """
        },
    )
    text = _ist_fehler(*_bau_ueber_cli(projekt, cli_projekte))
    assert "Narziss" in text


MEHRERE = """\
    from build123d import Box, Cylinder
    from werft import teil

    @teil("Quader", gruppe="Rumpf", material="PLA", farbe="grau", druck={"lage": "liegend"})
    def quader(dep):
        return Box(2, 3, 4)

    @teil("Rolle", haengt_von=["Quader"])
    def rolle(dep):
        assert dep["Quader"].volume > 0
        return Cylinder(1, 10)
    """


def test_mehrere_teile_in_einer_datei(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec: a file may contain several parts; all keyword arguments of @teil accepted.
    # Quader: 2*3*4 = 24 mm3; Rolle: pi * 1^2 * 10 = 31.4159... mm3.
    import math

    projekt = lege_projekt_an(tmp_path / "p", parts={"zwei.py": MEHRERE})
    dienst = dienst_starter(projekt)
    namen = sorted(t["name"] for t in dienst.status()["teile"])
    assert namen == ["Quader", "Rolle"]
    erg = dienst.bauen(None)
    assert sorted(erg["gebaut"]) == ["Quader", "Rolle"]
    assert nahe(erg["teile"]["Quader"]["volumen"], 24.0)
    assert nahe(erg["teile"]["Rolle"]["volumen"], math.pi * 10.0)


def test_rueckgabe_ohne_volumen_ist_baufehler(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec: the return value must be a build123d shape with volume > 0, otherwise a build error.
    projekt = lege_projekt_an(
        tmp_path / "p",
        parts={
            "falsch.py": """\
                from build123d import Box
                from werft import teil

                @teil("Zahl")
                def zahl(dep):
                    return 42

                @teil("Nichts")
                def nichts(dep):
                    return None

                @teil("Leer")
                def leer(dep):
                    # a box minus a larger box: empty compound, volume 0
                    return Box(1, 1, 1) - Box(2, 2, 2)

                @teil("Gut")
                def gut(dep):
                    return Box(1, 2, 3)
                """
        },
    )
    dienst = dienst_starter(projekt)
    erg = dienst.bauen(None)
    for name in ("Zahl", "Nichts", "Leer"):
        f = fehler_von(erg, name)
        assert isinstance(f.get("art"), str) and f["art"], f
    assert erg["teile"]["Gut"]["zustand"] == "gebaut"
    assert nahe(erg["teile"]["Gut"]["volumen"], 6.0)
