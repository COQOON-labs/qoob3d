"""Build graph and cache.

Spec: docs/m1_spec.md, section "Baugraph und Zwischenspeicher", Abnahme M1
items 2 and 3.

All volumes are boxes, so the expected values are products of the edge
lengths; tolerance 1e-6 relative (see conftest.nahe).
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any

from conftest import eintrag, lege_projekt_an, nahe, schreibe, toml_mit

A_QUELLE = """\
    from build123d import Box
    from werft import teil, soll

    @teil("A")
    def bauen(dep):
        k = soll("a.kante").wert
        return Box(k, k, k + {zusatz})
    """

B_QUELLE = """\
    from build123d import Box
    from werft import teil

    @teil("B", haengt_von=["A"])
    def bauen(dep):
        g = dep["A"].bounding_box().size
        return Box(g.X, g.Y, 1)
    """

C_QUELLE = """\
    from werft import teil
    from lib.form import platte

    @teil("C")
    def bauen(dep):
        return platte()
    """

D_QUELLE = """\
    from build123d import Box
    from werft import teil

    @teil("D")
    def bauen(dep):
        return Box(1, 2, 3)
    """

FORM_QUELLE = """\
    from build123d import Box

    BREITE = {breite}

    def platte():
        return Box(BREITE, 5, 6)
    """


def _soll_werte(kante: float, unbenutzt: float) -> list[dict]:
    return [eintrag("a.kante", kante), eintrag("a.unbenutzt", unbenutzt, "Vermutung", "annahme")]


def _graph_projekt(wurzel: Path) -> Path:
    return lege_projekt_an(
        wurzel,
        toml=toml_mit(anzahl=2),
        soll={"werte.json": _soll_werte(10, 1)},
        lib={"form.py": FORM_QUELLE.format(breite=4)},
        parts={
            "a.py": A_QUELLE.format(zusatz=0),
            "b.py": B_QUELLE,
            "c.py": C_QUELLE,
            "d.py": D_QUELLE,
        },
    )


def _volumen(erg: dict) -> dict[str, float]:
    return {n: t["volumen"] for n, t in erg["teile"].items()}


def _pruefe_volumen(erg: dict, erwartet: dict[str, float]) -> None:
    ist = _volumen(erg)
    for name, v in erwartet.items():
        assert nahe(ist[name], v), f"{name}: {ist[name]} != {v}"


def _pruefe_gebaut(erg: dict, gebaut: set[str], alle: set[str]) -> None:
    assert set(erg["gebaut"]) == gebaut, erg
    assert set(erg["aus_cache"]) == alle - gebaut, erg
    for name in alle:
        erwartet = "gebaut" if name in gebaut else "aus_cache"
        assert erg["teile"][name]["zustand"] == erwartet, (name, erg["teile"][name])


ALLE = {"A", "B", "C", "D"}


def test_abnahme_2_nur_geaenderte_teile_und_abhaengige(tmp_path: Path, dienst_starter: Any) -> None:
    # Abnahme 2. Volumes by hand:
    #   A = k * k * (k + zusatz); B = A.x * A.y * 1; C = BREITE * 5 * 6; D = 1 * 2 * 3
    projekt = _graph_projekt(tmp_path / "graph")
    dienst = dienst_starter(projekt)

    # 1. first build: all four built
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, ALLE, ALLE)
    _pruefe_volumen(erg, {"A": 1000.0, "B": 100.0, "C": 120.0, "D": 6.0})

    # 2. no change: all from cache, same volumes
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, set(), ALLE)
    _pruefe_volumen(erg, {"A": 1000.0, "B": 100.0, "C": 120.0, "D": 6.0})

    # 3. change A -> exactly A and B (B depends on A); A = 10 * 10 * 11
    schreibe(projekt / "parts" / "a.py", A_QUELLE.format(zusatz=1))
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, {"A", "B"}, ALLE)
    _pruefe_volumen(erg, {"A": 1100.0, "B": 100.0, "C": 120.0, "D": 6.0})

    # 4. change lib/form.py -> exactly C; C = 5 * 5 * 6
    schreibe(projekt / "lib" / "form.py", FORM_QUELLE.format(breite=5))
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, {"C"}, ALLE)
    _pruefe_volumen(erg, {"C": 150.0})

    # 5. change a soll value read by A -> A and B; A = 12 * 12 * 13, B = 12 * 12
    (projekt / "soll" / "werte.json").write_text(json.dumps(_soll_werte(12, 1)), encoding="utf-8")
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, {"A", "B"}, ALLE)
    _pruefe_volumen(erg, {"A": 1872.0, "B": 144.0})

    # 6. change a soll value nobody read, add a new soll file -> nothing
    (projekt / "soll" / "werte.json").write_text(json.dumps(_soll_werte(12, 2)), encoding="utf-8")
    (projekt / "soll" / "neu.json").write_text(json.dumps([eintrag("neu.x", 3)]), encoding="utf-8")
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, set(), ALLE)


def test_abnahme_3_bau_aus_cache_unter_halber_sekunde(tmp_path: Path, dienst_starter: Any) -> None:
    # Abnahme 3: a build entirely from the cache takes < 0.5 s (service warm).
    projekt = _graph_projekt(tmp_path / "graph")
    dienst = dienst_starter(projekt)
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, ALLE, ALLE)

    env, dauer = dienst.anfrage("POST", "/bauen", {"teile": None, "warten_s": 10})
    assert env["ok"] is True, env
    _pruefe_gebaut(env["ergebnis"], set(), ALLE)
    assert dauer < 0.5, f"cache build took {dauer:.3f} s"


def test_status_zustaende(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec /status: zustand "aktuell" | "veraltet" | "fehler" | "nie_gebaut".
    projekt = _graph_projekt(tmp_path / "graph")
    schreibe(
        projekt / "parts" / "e.py",
        """\
        from werft import teil

        @teil("E")
        def bauen(dep):
            raise RuntimeError("absichtlich")
        """,
    )
    dienst = dienst_starter(projekt)

    def zustaende() -> dict[str, str]:
        return {t["name"]: t["zustand"] for t in dienst.status()["teile"]}

    assert zustaende() == {n: "nie_gebaut" for n in ("A", "B", "C", "D", "E")}
    dateien = {t["name"]: str(t["datei"]) for t in dienst.status()["teile"]}
    assert dateien["A"].endswith("a.py") and dateien["C"].endswith("c.py"), dateien

    dienst.bauen(None)
    assert zustaende() == {"A": "aktuell", "B": "aktuell", "C": "aktuell", "D": "aktuell", "E": "fehler"}

    # A's file changes: A is stale and so is its dependent B.
    schreibe(projekt / "parts" / "a.py", A_QUELLE.format(zusatz=2))
    z = zustaende()
    assert z["A"] == "veraltet" and z["B"] == "veraltet", z
    assert z["C"] == "aktuell" and z["D"] == "aktuell", z


def test_cache_ablage_und_metadaten(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec: .werft/cache/<teil>/<schluessel>.brep + .json with volumen, bbox
    # [dx, dy, dz], gueltig, flaechen, soll_zugriffe, dauer_s, warnungen.
    # D = Box(1, 2, 3): volume 6, bbox [1, 2, 3], 6 faces, reads no soll value.
    projekt = _graph_projekt(tmp_path / "graph")
    dienst = dienst_starter(projekt)
    dienst.bauen(["D"])
    ordner = projekt / ".werft" / "cache" / "D"
    jsons = list(ordner.glob("*.json"))
    breps = list(ordner.glob("*.brep"))
    assert len(jsons) == 1 and len(breps) == 1, list(ordner.iterdir())
    assert jsons[0].stem == breps[0].stem, "brep and json must share the key"
    meta = json.loads(jsons[0].read_text(encoding="utf-8"))
    assert nahe(meta["volumen"], 6.0)
    assert len(meta["bbox"]) == 3
    for ist, soll in zip(meta["bbox"], [1.0, 2.0, 3.0]):
        # bbox of an axis-aligned box is exact up to the OCCT bbox gap (1e-7 order)
        assert abs(ist - soll) < 1e-3, meta["bbox"]
    assert meta["gueltig"] is True
    assert meta["flaechen"] == 6
    assert meta["soll_zugriffe"] == {}
    assert isinstance(meta["dauer_s"], (int, float)) and meta["dauer_s"] >= 0
    assert isinstance(meta["warnungen"], list)

    # Build result carries the same data (spec: volumen, bbox, gueltig, dauer_s)
    erg = dienst.bauen(["D"])
    d = erg["teile"]["D"]
    assert d["zustand"] == "aus_cache"
    assert nahe(d["volumen"], 6.0)
    assert d["gueltig"] is True
    assert [round(x, 3) for x in d["bbox"]] == [1.0, 2.0, 3.0]
    assert isinstance(d["dauer_s"], (int, float))


def test_abhaengigkeiten_werden_mitgebaut(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec POST /bauen: builds the named parts plus the dependencies they need,
    # nothing else.
    projekt = _graph_projekt(tmp_path / "graph")
    dienst = dienst_starter(projekt)
    erg = dienst.bauen(["B"])
    assert set(erg["gebaut"]) == {"A", "B"}, erg
    assert "C" not in erg["teile"] and "D" not in erg["teile"], erg
    assert nahe(erg["teile"]["B"]["volumen"], 100.0)


def test_alle_teile_einer_datei_werden_neu_gebaut(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec: if file A in parts/ changes, all parts of that file are rebuilt.
    quelle = """\
        from build123d import Box
        from werft import teil

        @teil("P1")
        def p1(dep):
            return Box(1, 1, 1)

        @teil("P2")
        def p2(dep):
            return Box(2, 2, 2)
        {kommentar}
        """
    projekt = lege_projekt_an(
        tmp_path / "p",
        parts={"paar.py": quelle.format(kommentar=""), "allein.py": D_QUELLE},
    )
    dienst = dienst_starter(projekt)
    erg = dienst.bauen(None)
    assert set(erg["gebaut"]) == {"P1", "P2", "D"}
    # Only a comment changes, but the content of the file is different.
    schreibe(projekt / "parts" / "paar.py", quelle.format(kommentar="# geaendert"))
    erg = dienst.bauen(None)
    assert set(erg["gebaut"]) == {"P1", "P2"}, erg
    assert set(erg["aus_cache"]) == {"D"}, erg


def test_lib_importarten_und_transitiv(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec item 2: lib files imported directly or transitively via
    # `import lib.x`, `from lib.x import y`, `from lib import x` (AST).
    # Each part's volume = MASS ** 3 of the module it uses.
    projekt = lege_projekt_an(
        tmp_path / "p",
        lib={
            "basis.py": "MASS = 2\n",
            "mitte.py": "from lib.basis import MASS\nDOPPELT = MASS\n",
            "eins.py": "MASS = 3\n",
            "zwei.py": "MASS = 4\n",
        },
        parts={
            "import_modul.py": """\
                import lib.eins
                from build123d import Box
                from werft import teil

                @teil("ImportModul")
                def bauen(dep):
                    m = lib.eins.MASS
                    return Box(m, m, m)
                """,
            "from_lib.py": """\
                from lib import zwei
                from build123d import Box
                from werft import teil

                @teil("FromLib")
                def bauen(dep):
                    m = zwei.MASS
                    return Box(m, m, m)
                """,
            "transitiv.py": """\
                from lib.mitte import DOPPELT
                from build123d import Box
                from werft import teil

                @teil("Transitiv")
                def bauen(dep):
                    return Box(DOPPELT, DOPPELT, DOPPELT)
                """,
        },
    )
    alle = {"ImportModul", "FromLib", "Transitiv"}
    dienst = dienst_starter(projekt)
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, alle, alle)
    _pruefe_volumen(erg, {"ImportModul": 27.0, "FromLib": 64.0, "Transitiv": 8.0})

    # transitive: lib/basis.py is imported only through lib/mitte.py
    (projekt / "lib" / "basis.py").write_text("MASS = 5\n", encoding="utf-8")
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, {"Transitiv"}, alle)
    _pruefe_volumen(erg, {"Transitiv": 125.0})

    (projekt / "lib" / "eins.py").write_text("MASS = 6\n", encoding="utf-8")
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, {"ImportModul"}, alle)
    _pruefe_volumen(erg, {"ImportModul": 216.0})

    (projekt / "lib" / "zwei.py").write_text("MASS = 7\n", encoding="utf-8")
    erg = dienst.bauen(None)
    _pruefe_gebaut(erg, {"FromLib"}, alle)
    _pruefe_volumen(erg, {"FromLib": 343.0})


def test_cache_ueberlebt_neustart_des_dienstes(tmp_path: Path, dienst_starter: Any) -> None:
    # The cache lives on disk (.werft/cache), so a restarted service serves it.
    from conftest import raeume_dienst_auf

    projekt = _graph_projekt(tmp_path / "graph")
    dienst = dienst_starter(projekt)
    _pruefe_gebaut(dienst.bauen(None), ALLE, ALLE)
    raeume_dienst_auf(projekt)
    time.sleep(0.2)
    dienst = dienst_starter(projekt)
    _pruefe_gebaut(dienst.bauen(None), set(), ALLE)
