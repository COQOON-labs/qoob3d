"""Helper library werft.hilfen (rules from M0).

Spec: docs/m1_spec.md, section "Hilfsbibliothek werft.hilfen".
Runs in the test process (needs build123d, no service).
"""

from __future__ import annotations

import math

import pytest
from build123d import Box, Cylinder, Pos

import werft
from werft.hilfen import draht, loch, pruefe_volumen, strahl, zapfen

# Cylinders and boxes: OCCT volumes are exact to floating rounding, 1e-9
# relative leaves several orders of magnitude of headroom.
REL_EXAKT = 1e-9
# Coordinates of bounding boxes: OCCT may enlarge a bbox by its gap (~1e-7).
ABS_BBOX = 1e-4


def _bbox(form):
    bb = form.bounding_box()
    return bb.min, bb.max


# ---- draht ----------------------------------------------------------------------


@pytest.mark.parametrize("biegeradius", [1.0, 0.5, 0.0, -1.0])
def test_draht_biegeradius_zu_klein(biegeradius: float) -> None:
    # Spec: ValueError if biegeradius <= d/2 (here d/2 = 1.0, boundary included).
    with pytest.raises(ValueError):
        draht([(0, 0, 0), (20, 0, 0), (20, 20, 0)], d=2.0, biegeradius=biegeradius)


def test_draht_volumen_nach_pappus() -> None:
    # L-shaped wire, d = 2 (r = 1), bend radius R = 5.
    # Centre line: two straights of 20 - R = 15 each, plus a quarter arc
    # 2 * pi * R / 4 = 2.5 * pi -> L = 30 + 2.5 * pi.
    # Volume of a swept circle along a planar line/arc path = pi * r^2 * L
    # (Pappus for the torus section) = pi * (30 + 2.5 * pi) = 118.9218 mm3.
    # Tolerance 1e-4: an implementation may approximate the swept surfaces by
    # B-splines; an error in the geometry (missing arc, wrong radius) is > 1 %.
    form = draht([(0, 0, 0), (20, 0, 0), (20, 20, 0)], d=2.0, biegeradius=5.0)
    erwartet = math.pi * 1.0**2 * (30.0 + 2.5 * math.pi)
    assert form.is_valid
    assert form.volume == pytest.approx(erwartet, rel=1e-4)
    mn, mx = _bbox(form)
    # x from -0 (start cap) to 20 + r, y from -r to 20, z from -r to r
    assert mx.X == pytest.approx(21.0, abs=ABS_BBOX)
    assert mn.Y == pytest.approx(-1.0, abs=ABS_BBOX)
    assert mx.Y == pytest.approx(20.0, abs=ABS_BBOX)
    assert mn.Z == pytest.approx(-1.0, abs=ABS_BBOX)
    assert mx.Z == pytest.approx(1.0, abs=ABS_BBOX)


def test_draht_dreidimensional() -> None:
    # Three straights of 10 each with two 90 degree bends of R = 3 in 3D, d = 1.
    # L = 30 - 4 * R + 2 * (pi / 2) * R = 18 + 3 * pi; V = pi * 0.25 * L.
    form = draht([(0, 0, 0), (10, 0, 0), (10, 10, 0), (10, 10, 10)], d=1.0, biegeradius=3.0)
    erwartet = math.pi * 0.25 * (18.0 + 3.0 * math.pi)
    assert form.volume == pytest.approx(erwartet, rel=1e-4)


# ---- zapfen / loch --------------------------------------------------------------


def test_zapfen_masse() -> None:
    # Spec: cylinder from z = 0 towards -Z. V = pi * d^2 / 4 * l.
    d, l = 4.0, 2.45
    z = zapfen(d=d, l=l)
    assert z.volume == pytest.approx(math.pi * d * d / 4 * l, rel=REL_EXAKT)
    mn, mx = _bbox(z)
    assert mx.Z == pytest.approx(0.0, abs=ABS_BBOX)
    assert mn.Z == pytest.approx(-l, abs=ABS_BBOX)
    assert mx.X - mn.X == pytest.approx(d, abs=ABS_BBOX)
    assert mx.Y - mn.Y == pytest.approx(d, abs=ABS_BBOX)


def test_loch_mit_standardspiel() -> None:
    # Spec: diameter d + 2 * spiel, depth l + spiel, spiel defaults to 0.2.
    # d = 4, l = 2.45 -> D = 4.4, T = 2.65; V = pi * 4.4^2 / 4 * 2.65.
    h = loch(d=4.0, l=2.45)
    assert h.volume == pytest.approx(math.pi * 4.4**2 / 4 * 2.65, rel=REL_EXAKT)
    mn, mx = _bbox(h)
    assert mx.Z == pytest.approx(0.0, abs=ABS_BBOX)
    assert mn.Z == pytest.approx(-2.65, abs=ABS_BBOX)
    assert mx.X - mn.X == pytest.approx(4.4, abs=ABS_BBOX)
    assert mx.Y - mn.Y == pytest.approx(4.4, abs=ABS_BBOX)


@pytest.mark.parametrize("spiel", [0.0, 0.1, 0.5])
def test_loch_mit_spiel(spiel: float) -> None:
    d, l = 3.0, 5.0
    h = loch(d=d, l=l, spiel=spiel)
    durchmesser = d + 2 * spiel
    tiefe = l + spiel
    assert h.volume == pytest.approx(math.pi * durchmesser**2 / 4 * tiefe, rel=REL_EXAKT)
    mn, mx = _bbox(h)
    assert mn.Z == pytest.approx(-tiefe, abs=ABS_BBOX)
    assert mx.Z == pytest.approx(0.0, abs=ABS_BBOX)
    assert mx.X - mn.X == pytest.approx(durchmesser, abs=ABS_BBOX)


def test_zapfen_passt_in_loch() -> None:
    # Pin and hole with the same d, l: the pin lies entirely inside the hole.
    z = zapfen(d=4.0, l=2.45)
    h = loch(d=4.0, l=2.45)
    rest = z - h
    assert rest.volume == pytest.approx(0.0, abs=1e-6)


# ---- strahl --------------------------------------------------------------------


def _als_tupel(treffer) -> list[tuple[float, float, float]]:
    out = []
    for p in treffer:
        t = tuple(float(c) for c in p)
        assert len(t) == 3
        out.append(t)
    return out


def _gleich(ist, soll, tol: float = 1e-6) -> bool:
    return len(ist) == len(soll) and all(
        all(abs(a - b) <= tol for a, b in zip(p, q)) for p, q in zip(ist, soll)
    )


def test_strahl_durch_wuerfel() -> None:
    # Box 10 x 10 x 10 centred at the origin; ray from z = -100 upwards along
    # the z axis hits the faces z = -5 and z = 5, in this order.
    treffer = _als_tupel(strahl(Box(10, 10, 10), (0, 0, -100), (0, 0, 1)))
    assert _gleich(treffer, [(0.0, 0.0, -5.0), (0.0, 0.0, 5.0)]), treffer


def test_strahl_sortiert_nach_abstand_bei_bohrung() -> None:
    # Box 10^3 with a through bore of radius 2 along z. A ray along -X from
    # x = 100 through the centre hits x = 5, 2, -2, -5 (sorted by distance
    # from the start, not by coordinate).
    form = Box(10, 10, 10) - Cylinder(2, 20)
    treffer = _als_tupel(strahl(form, (100, 0, 0), (-1, 0, 0)))
    assert _gleich(treffer, [(5.0, 0, 0), (2.0, 0, 0), (-2.0, 0, 0), (-5.0, 0, 0)]), treffer


def test_strahl_schraeg_und_verschoben() -> None:
    # Box 2 x 2 x 2 moved to (10, 0, 0); ray from (0, 0, 0) along +X hits
    # x = 9 and x = 11. Direction given unnormalised (5, 0, 0).
    form = Pos(10, 0, 0) * Box(2, 2, 2)
    treffer = _als_tupel(strahl(form, (0, 0, 0), (5, 0, 0)))
    assert _gleich(treffer, [(9.0, 0, 0), (11.0, 0, 0)]), treffer


def test_strahl_ohne_treffer() -> None:
    treffer = list(strahl(Box(10, 10, 10), (100, 100, -100), (0, 0, 1)))
    assert treffer == []


# ---- pruefe_volumen -------------------------------------------------------------
# Ambiguity: the spec does not say whether vorher/nachher are shapes or
# volumes. The helper is meant to be called with the operands of a build123d
# operation, so shapes are passed here.


def test_plausibilitaetsfehler_ist_exception() -> None:
    assert issubclass(werft.PlausibilitaetsFehler, Exception)


def test_abziehen_plausibel() -> None:
    vorher = Box(10, 10, 10)
    nachher = vorher - Cylinder(2, 20)  # 1000 - pi * 4 * 10
    pruefe_volumen(vorher, nachher, "abziehen")


def test_abziehen_ergibt_null_volumen() -> None:
    # Box minus a larger box: empty result, volume 0 <= 1e-6.
    vorher = Box(10, 10, 10)
    nachher = vorher - Box(20, 20, 20)
    with pytest.raises(werft.PlausibilitaetsFehler):
        pruefe_volumen(vorher, nachher, "abziehen")


def test_abziehen_wird_nicht_kleiner() -> None:
    # The tool body misses the part: volume unchanged -> implausible.
    vorher = Box(10, 10, 10)
    nachher = vorher - Pos(100, 0, 0) * Box(1, 1, 1)
    with pytest.raises(werft.PlausibilitaetsFehler):
        pruefe_volumen(vorher, nachher, "abziehen")


def test_abziehen_wird_groesser() -> None:
    vorher = Box(10, 10, 10)
    nachher = Box(11, 10, 10)
    with pytest.raises(werft.PlausibilitaetsFehler):
        pruefe_volumen(vorher, nachher, "abziehen")


def test_vereinigen_plausibel() -> None:
    vorher = Box(10, 10, 10)
    nachher = vorher + Pos(6, 0, 0) * Box(4, 4, 4)  # grows by 2 * 4 * 4 = 32
    pruefe_volumen(vorher, nachher, "vereinigen")


def test_vereinigen_bleibt_gleich() -> None:
    # The added body lies completely inside: volume stays 1000 -> implausible.
    vorher = Box(10, 10, 10)
    nachher = vorher + Box(2, 2, 2)
    with pytest.raises(werft.PlausibilitaetsFehler):
        pruefe_volumen(vorher, nachher, "vereinigen")


def test_vereinigen_schrumpft() -> None:
    with pytest.raises(werft.PlausibilitaetsFehler):
        pruefe_volumen(Box(10, 10, 10), Box(9, 10, 10), "vereinigen")


def test_schale_plausibel() -> None:
    vorher = Box(10, 10, 10)
    nachher = vorher - Box(8, 8, 8)  # hollow, 1000 - 512 = 488
    pruefe_volumen(vorher, nachher, "schale")


def test_schale_unveraendert() -> None:
    # M0: offset with Kind.INTERSECTION returned the body unchanged.
    vorher = Box(10, 10, 10)
    with pytest.raises(werft.PlausibilitaetsFehler):
        pruefe_volumen(vorher, Box(10, 10, 10), "schale")


def test_schale_fast_null_volumen() -> None:
    # M0: Kind.ARC returned a "valid" body with 8.5e-14 mm3. Here a body of
    # 1e-9 mm3 (0.001^3): smaller than before, but <= 1e-6 -> implausible.
    with pytest.raises(werft.PlausibilitaetsFehler):
        pruefe_volumen(Box(10, 10, 10), Box(0.001, 0.001, 0.001), "schale")


def test_schale_knapp_ueber_grenze() -> None:
    # 0.02^3 = 8e-6 mm3 > 1e-6: the zero-volume rule must not fire.
    pruefe_volumen(Box(10, 10, 10), Box(0.02, 0.02, 0.02), "schale")
