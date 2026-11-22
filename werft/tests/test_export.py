"""Export to out/<teil>.step|stl|3mf.

Spec: docs/m1_spec.md, POST /export and `werft export`.

The files are checked by independent readers: STEP via build123d's importer
in the test process, STL and 3MF by a small parser written here that sums the
signed tetrahedron volumes of the triangles (divergence theorem).
Kiste = Box(10, 20, 30) -> volume 6000 mm3, bbox 10 x 20 x 30.
"""

from __future__ import annotations

import re
import struct
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path
from typing import Any

import pytest

from conftest import geteilter_dienst, lege_projekt_an, nahe, werft_cli

KISTE = """\
    from build123d import Box
    from werft import teil

    @teil("Kiste")
    def bauen(dep):
        return Box(10, 20, 30)
    """
ROLLE = """\
    from build123d import Cylinder
    from werft import teil

    @teil("Rolle")
    def bauen(dep):
        return Cylinder(5, 10)
    """
KISTE_VOLUMEN = 6000.0


def _dreiecke_volumen(dreiecke: list[tuple[tuple[float, ...], ...]]) -> float:
    # V = 1/6 * sum(v0 . (v1 x v2)) over a closed, consistently oriented mesh.
    v = 0.0
    for a, b, c in dreiecke:
        v += (
            a[0] * (b[1] * c[2] - b[2] * c[1])
            - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0])
        )
    return abs(v) / 6.0


def _bbox(dreiecke: list[tuple[tuple[float, ...], ...]]) -> list[float]:
    punkte = [p for d in dreiecke for p in d]
    return [max(p[i] for p in punkte) - min(p[i] for p in punkte) for i in range(3)]


def _stl_dreiecke(pfad: Path) -> list[tuple[tuple[float, ...], ...]]:
    daten = pfad.read_bytes()
    if len(daten) >= 84:
        n = struct.unpack_from("<I", daten, 80)[0]
        if len(daten) == 84 + 50 * n:
            out = []
            for i in range(n):
                w = struct.unpack_from("<12f", daten, 84 + 50 * i)
                out.append((w[3:6], w[6:9], w[9:12]))
            return out
    text = daten.decode("ascii")
    assert text.lstrip().startswith("solid"), "neither binary nor ASCII STL"
    zahlen = re.findall(r"vertex\s+(\S+)\s+(\S+)\s+(\S+)", text)
    ecken = [tuple(float(x) for x in z) for z in zahlen]
    assert len(ecken) % 3 == 0
    return [tuple(ecken[i : i + 3]) for i in range(0, len(ecken), 3)]


def _3mf_dreiecke(pfad: Path) -> list[tuple[tuple[float, ...], ...]]:
    with zipfile.ZipFile(pfad) as z:
        assert z.testzip() is None, "corrupt 3MF zip"
        namen = z.namelist()
        assert "[Content_Types].xml" in namen
        assert "_rels/.rels" in namen
        modelle = [n for n in namen if n.lower().endswith(".model")]
        assert modelle, namen
        out = []
        for name in modelle:
            wurzel = ET.fromstring(z.read(name))
            einheit = wurzel.get("unit")
            assert einheit in (None, "millimeter"), einheit
            for mesh in wurzel.iter():
                if not mesh.tag.endswith("}mesh"):
                    continue
                ecken = [
                    (float(v.get("x")), float(v.get("y")), float(v.get("z")))
                    for v in mesh.iter()
                    if v.tag.endswith("}vertex")
                ]
                for t in mesh.iter():
                    if t.tag.endswith("}triangle"):
                        out.append(
                            (ecken[int(t.get("v1"))], ecken[int(t.get("v2"))], ecken[int(t.get("v3"))])
                        )
        return out


def _projekt(tmp_path: Path) -> Path:
    return lege_projekt_an(tmp_path / "p", parts={"kiste.py": KISTE, "rolle.py": ROLLE})


@pytest.fixture(scope="module")
def dienst(tmp_path_factory: Any) -> Any:
    # One service for the HTTP export tests (runtime budget < 120 s). Only
    # test_export_mehrere_teile_baut_vorher touches "Rolle", so it is unbuilt there.
    yield from geteilter_dienst(tmp_path_factory, "export", parts={"kiste.py": KISTE, "rolle.py": ROLLE})


def test_export_step(dienst: Any) -> None:
    from build123d import import_step

    projekt = dienst.projekt
    env = dienst.post("/export", {"teile": ["Kiste"], "format": "step"})
    assert env["ok"] is True, env
    pfad = projekt / "out" / "Kiste.step"
    assert pfad.is_file()
    assert pfad.read_text(encoding="ascii", errors="replace").startswith("ISO-10303-21")
    form = import_step(str(pfad))
    # STEP keeps the exact B-rep: volume as analytic (1e-6, see conftest.nahe)
    assert nahe(form.volume, KISTE_VOLUMEN)


def test_export_stl(dienst: Any) -> None:
    projekt = dienst.projekt
    env = dienst.post("/export", {"teile": ["Kiste"], "format": "stl"})
    assert env["ok"] is True, env
    dreiecke = _stl_dreiecke(projekt / "out" / "Kiste.stl")
    # A box triangulates exactly (12 triangles minimum); its corner coordinates
    # are representable in float32, so the volume is exact to rounding.
    assert len(dreiecke) >= 12
    assert nahe(_dreiecke_volumen(dreiecke), KISTE_VOLUMEN, rel=1e-5)
    assert [round(x, 3) for x in _bbox(dreiecke)] == [10.0, 20.0, 30.0]


def test_export_3mf(dienst: Any) -> None:
    projekt = dienst.projekt
    env = dienst.post("/export", {"teile": ["Kiste"], "format": "3mf"})
    assert env["ok"] is True, env
    dreiecke = _3mf_dreiecke(projekt / "out" / "Kiste.3mf")
    assert len(dreiecke) >= 12
    assert nahe(_dreiecke_volumen(dreiecke), KISTE_VOLUMEN, rel=1e-5)
    assert [round(x, 3) for x in _bbox(dreiecke)] == [10.0, 20.0, 30.0]


def test_export_mehrere_teile_baut_vorher(dienst: Any) -> None:
    # Spec: export builds first if needed (nothing was built before here).
    # Rolle = Cylinder(5, 10): STL is a faceted approximation, its volume is
    # smaller than pi * 25 * 10 = 785.4 but more than the inscribed 12-gon
    # prism (3 * r^2 * h = 750), any sane tessellation is finer than that.
    projekt = dienst.projekt
    env = dienst.post("/export", {"teile": ["Kiste", "Rolle"], "format": "stl"})
    assert env["ok"] is True, env
    assert nahe(_dreiecke_volumen(_stl_dreiecke(projekt / "out" / "Kiste.stl")), KISTE_VOLUMEN, rel=1e-5)
    v_rolle = _dreiecke_volumen(_stl_dreiecke(projekt / "out" / "Rolle.stl"))
    assert 750.0 < v_rolle <= 3.14159266 * 25 * 10 * (1 + 1e-6), v_rolle
    st = {t["name"]: t["zustand"] for t in dienst.status()["teile"]}
    assert st["Kiste"] == "aktuell" and st["Rolle"] == "aktuell"


def test_export_unbekanntes_format(dienst: Any) -> None:
    projekt = dienst.projekt
    env = dienst.post("/export", {"teile": ["Kiste"], "format": "obj"})
    assert env["ok"] is False, env
    assert not (projekt / "out" / "Kiste.obj").exists()


def test_cli_export(tmp_path: Path, cli_projekte: list[Path]) -> None:
    # Spec CLI: `werft export TEIL ... --format step|stl|3mf`; builds first if needed.
    projekt = _projekt(tmp_path)
    cli_projekte.append(projekt)
    for fmt in ("step", "stl", "3mf"):
        erg = werft_cli(["-C", str(projekt), "export", "Kiste", "--format", fmt])
        assert erg.code == 0, erg
        assert erg.json()["ok"] is True
        pfad = projekt / "out" / f"Kiste.{fmt}"
        assert pfad.is_file() and pfad.stat().st_size > 0, fmt
    assert nahe(_dreiecke_volumen(_stl_dreiecke(projekt / "out" / "Kiste.stl")), KISTE_VOLUMEN, rel=1e-5)
