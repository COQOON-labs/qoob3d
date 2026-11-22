"""Sollwerte: loading, validation, annahmen(), soll() inside a build.

Spec: docs/m1_spec.md, section "Sollwerte (soll/*.json)", Abnahme M1 item 4.

Test seam for validation: the spec names no public loader, but
`werft.annahmen(projekt)` loads all soll/*.json files and "Fehler beim Laden"
must raise `werft.SollFehler`. These tests therefore trigger validation
through annahmen().
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

import werft
from conftest import eintrag, lege_projekt_an, nahe, werft_cli


def _id(e: Any) -> str:
    """annahmen() returns 'entries'; the spec does not fix dict vs. object."""
    return e["id"] if isinstance(e, dict) else e.id


def _projekt(tmp_path: Path, soll: dict[str, Any]) -> Path:
    return lege_projekt_an(tmp_path / "p", soll=soll)


def _lade_fehler(tmp_path: Path, soll: dict[str, Any]) -> str:
    projekt = _projekt(tmp_path, soll)
    with pytest.raises(werft.SollFehler) as info:
        werft.annahmen(projekt)
    return str(info.value)


# ---- Abnahme M1 item 4 -----------------------------------------------------


def test_abnahme_4_soll_ohne_quelle_wirft_sollfehler_mit_dateiname(tmp_path: Path) -> None:
    # Abnahme 4: entry without "quelle" -> SollFehler naming the file (and the id).
    kaputt = {"id": "kran.K1", "wert": {"d": 13, "h": 20}, "status": "gemessen"}
    meldung = _lade_fehler(tmp_path, {"kranmasse.json": [kaputt]})
    assert "kranmasse.json" in meldung
    assert "kran.K1" in meldung


def test_sollfehler_ist_exception() -> None:
    assert issubclass(werft.SollFehler, Exception)


# ---- validation of single fields -------------------------------------------


@pytest.mark.parametrize(
    "kaputt, grund",
    [
        ({"id": "x.leer", "wert": 1, "quelle": "", "status": "gemessen"}, "empty quelle"),
        ({"id": "x.ohnewert", "quelle": "Foto", "status": "gemessen"}, "missing wert"),
        ({"id": "x.ohnestatus", "wert": 1, "quelle": "Foto"}, "missing status"),
        ({"id": "x.status", "wert": 1, "quelle": "Foto", "status": "geschaetzt"}, "unknown status"),
        ({"id": "x.text", "wert": "13", "quelle": "Foto", "status": "gemessen"}, "wert is a string"),
        ({"id": "x.liste", "wert": [1, 2], "quelle": "Foto", "status": "gemessen"}, "wert is a list"),
        (
            {"id": "x.objtext", "wert": {"d": "dreizehn"}, "quelle": "Foto", "status": "gemessen"},
            "object wert with non-number",
        ),
    ],
)
def test_ungueltiger_eintrag_nennt_datei_und_id(tmp_path: Path, kaputt: dict, grund: str) -> None:
    # Spec: required fields id, wert (number or object of numbers), quelle (not
    # empty), status in {gemessen, eine_quelle, annahme}. Error names file and id.
    gut = eintrag("x.gut", 5)
    meldung = _lade_fehler(tmp_path, {"werte.json": [gut, kaputt]})
    assert "werte.json" in meldung, grund
    assert kaputt["id"] in meldung, grund


def test_eintrag_ohne_id_nennt_datei_und_index(tmp_path: Path) -> None:
    # Spec: without id the message contains the index of the entry. The entry
    # sits at index 5 (0-based; 1-based it would be 6). File name and other ids
    # contain no digits, so a digit in the message can only come from the index.
    gute = [eintrag(f"teil.{n}", 1) for n in ("a", "b", "c", "d", "e")]
    ohne_id = {"wert": 1, "quelle": "Foto", "status": "gemessen"}
    meldung = _lade_fehler(tmp_path, {"masse.json": [*gute, ohne_id]})
    assert "masse.json" in meldung
    # Ambiguity: "Index" is read as 0-based; 1-based numbering is tolerated.
    assert "5" in meldung or "6" in meldung, meldung


def test_doppelte_id_ueber_zwei_dateien(tmp_path: Path) -> None:
    # Spec: id unique over all files.
    meldung = _lade_fehler(
        tmp_path,
        {
            "erste.json": [eintrag("rumpf.laenge", 100)],
            "zweite.json": [eintrag("rumpf.laenge", 101)],
        },
    )
    assert "rumpf.laenge" in meldung
    assert "erste.json" in meldung or "zweite.json" in meldung


def test_doppelte_id_in_einer_datei(tmp_path: Path) -> None:
    meldung = _lade_fehler(tmp_path, {"eine.json": [eintrag("a.b", 1), eintrag("a.b", 2)]})
    assert "a.b" in meldung
    assert "eine.json" in meldung


def test_datei_ist_keine_liste(tmp_path: Path) -> None:
    # Spec: every file is a JSON list of entries.
    meldung = _lade_fehler(tmp_path, {"objekt.json": eintrag("a.b", 1)})
    assert "objekt.json" in meldung


def test_ungueltiges_json(tmp_path: Path) -> None:
    meldung = _lade_fehler(tmp_path, {"defekt.json": '[{"id": "a.b", "wert": 1,'})
    assert "defekt.json" in meldung


# ---- valid files and annahmen() -------------------------------------------


def test_gueltige_werte_laden_ohne_fehler(tmp_path: Path) -> None:
    # All three status values, int/float/object values, entries spread over files.
    projekt = _projekt(
        tmp_path,
        {
            "a.json": [
                eintrag("a.int", 13, status="gemessen"),
                eintrag("a.float", 2.45, status="eine_quelle"),
            ],
            "b.json": [eintrag("b.obj", {"d": 13, "h": 20.5}, status="gemessen")],
        },
    )
    assert list(werft.annahmen(projekt)) == []


def test_annahmen_liefert_genau_die_annahmen(tmp_path: Path) -> None:
    # Spec: annahmen(projekt) -> list of all entries with status "annahme".
    projekt = _projekt(
        tmp_path,
        {
            "a.json": [
                eintrag("a.gemessen", 1, status="gemessen"),
                eintrag("a.annahme", 2, quelle="Schaetzung", status="annahme"),
            ],
            "b.json": [
                eintrag("b.eine", 3, status="eine_quelle"),
                eintrag("b.annahme", {"d": 4}, quelle="Vermutung", status="annahme"),
            ],
        },
    )
    ids = sorted(_id(e) for e in werft.annahmen(projekt))
    assert ids == ["a.annahme", "b.annahme"]


def test_annahmen_ohne_soll_dateien_ist_leer(tmp_path: Path) -> None:
    projekt = lege_projekt_an(tmp_path / "leer")
    assert list(werft.annahmen(projekt)) == []


def test_cli_annahmen_lokal_ohne_dienst(tmp_path: Path) -> None:
    # Spec: `werft annahmen` runs locally, without the service -> no
    # .werft/dienst.json may appear. Output is the JSON envelope, exit 0.
    projekt = _projekt(
        tmp_path,
        {"a.json": [eintrag("a.gemessen", 1), eintrag("a.vermutet", 2, "Bauchgefuehl", "annahme")]},
    )
    erg = werft_cli(["-C", str(projekt), "annahmen"])
    assert erg.code == 0, erg
    env = erg.json()
    assert env["ok"] is True
    text = json.dumps(env["ergebnis"])
    assert "a.vermutet" in text
    assert "a.gemessen" not in text
    assert not (projekt / ".werft" / "dienst.json").exists()


def test_cli_annahmen_mit_kaputter_datei_exit_1(tmp_path: Path) -> None:
    projekt = _projekt(tmp_path, {"kaputt.json": [{"id": "k.x", "wert": 1, "status": "gemessen"}]})
    erg = werft_cli(["-C", str(projekt), "annahmen"])
    assert erg.code == 1, erg
    env = erg.json()
    assert env["ok"] is False
    assert isinstance(env["fehler"]["art"], str)
    assert "kaputt.json" in env["fehler"]["meldung"]


# ---- soll() inside a build ------------------------------------------------

SOLL_TEIL = """\
    from build123d import Box
    from werft import teil, soll

    @teil("Kiste")
    def bauen(dep):
        m = soll("kiste.masse")
        # object value: attribute access plus .wert as dict
        assert m.wert == {"dx": 10, "dy": 20, "dz": 5}, m.wert
        assert (m.dx, m.dy, m.dz) == (10, 20, 5)
        assert m.quelle == "Zeichnung Blatt 3"
        assert m.status == "gemessen"
        w = soll("kiste.wand")
        # number value: .wert
        assert w.wert == 2
        assert w.quelle == "Schaetzung"
        assert w.status == "annahme"
        return Box(m.dx, m.dy, m.dz - w.wert)
    """


def test_soll_attributzugriff_und_protokoll(tmp_path: Path, dienst_starter: Any) -> None:
    # Spec: soll(id) -> .wert (dict for objects) + attribute access, .quelle,
    # .status; every read during a build is logged (id and value) and stored in
    # the cache metadata as soll_zugriffe {id: wert}.
    # Expected volume: 10 * 20 * (5 - 2) = 600 mm3.
    projekt = lege_projekt_an(
        tmp_path / "p",
        soll={
            "kiste.json": [
                eintrag("kiste.masse", {"dx": 10, "dy": 20, "dz": 5}, "Zeichnung Blatt 3"),
                eintrag("kiste.wand", 2, "Schaetzung", "annahme"),
                eintrag("kiste.ungelesen", 99),
            ]
        },
        parts={"kiste.py": SOLL_TEIL},
    )
    dienst = dienst_starter(projekt)
    erg = dienst.bauen(["Kiste"])
    k = erg["teile"]["Kiste"]
    assert k["zustand"] == "gebaut", k
    assert nahe(k["volumen"], 600.0)

    metas = list((projekt / ".werft" / "cache" / "Kiste").glob("*.json"))
    assert len(metas) == 1, metas
    meta = json.loads(metas[0].read_text(encoding="utf-8"))
    assert meta["soll_zugriffe"] == {"kiste.masse": {"dx": 10, "dy": 20, "dz": 5}, "kiste.wand": 2}


def test_kaputte_solldatei_beim_bau_nennt_datei(tmp_path: Path, dienst_starter: Any) -> None:
    # A SollFehler must reach the caller of a build with the file name.
    projekt = lege_projekt_an(
        tmp_path / "p",
        soll={"kaputt.json": [{"id": "k.x", "wert": 1, "status": "gemessen"}]},
        parts={
            "k.py": """\
                from build123d import Box
                from werft import teil, soll

                @teil("K")
                def bauen(dep):
                    return Box(soll("k.x").wert, 1, 1)
                """
        },
    )
    dienst = dienst_starter(projekt)
    env = dienst.bauen_env(["K"], warten_s=60)
    if env["ok"]:
        erg = env["ergebnis"]
        if "auftrag" in erg and "teile" not in erg:
            erg = dienst.warte_auftrag(erg["auftrag"])["ergebnis"]
        assert erg["teile"]["K"]["zustand"] == "fehler", erg
        assert "kaputt.json" in json.dumps(erg["teile"]["K"]["fehler"], ensure_ascii=False)
    else:
        assert "kaputt.json" in env["fehler"]["meldung"], env
