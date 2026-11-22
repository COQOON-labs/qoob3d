"""Schwachstellen des Geometriekerns (OCCT ueber build123d) gezielt pruefen.

Jeder Fall laeuft in einem eigenen Prozess mit Zeitgrenze. Ergebnis:
out/m0_schwaechen.json.

    uv run python m0/schwaechen.py
"""

from __future__ import annotations

import json
import multiprocessing as mp
import sys
import time
import traceback
from pathlib import Path

HIER = Path(__file__).resolve().parent
OUT = HIER.parent / "out"
CACHE = HIER.parent / ".werft" / "cache"
GRENZE_S = 90


def _ergebnis(teil, soll_vol=None):
    if teil is None:
        return dict(ok=False, fehler="Ergebnis ist None")
    v = teil.volume
    erg = dict(
        ok=bool(teil.is_valid) and v > 0,
        gueltig=bool(teil.is_valid),
        volumen=v,
        koerper=len(teil.solids()),
        flaechen=len(teil.faces()),
    )
    if soll_vol:
        erg["soll"] = soll_vol
        erg["abw_prozent"] = 100 * (v - soll_vol) / soll_vol
    return erg


# --- Faelle (laufen im Kindprozess) ----------------------------------------
def fall_kransaeule_alle_kanten_runden():
    import teile
    from build123d import fillet

    k = teile.kransaeule()
    return _ergebnis(fillet(k.edges(), 0.2))


def fall_reling_knoten_runden():
    """Verrundung an den Durchdringungskanten duenner Draehte (Knoten)."""
    import teile
    from build123d import fillet

    r = teile.reling_verschmolzen()
    # Kanten der Durchdringungen: weder Kreis an einem Rohrende noch Naht.
    kanten = [e for e in r.edges() if e.geom_type.name == "BSPLINE" or e.geom_type.name == "OTHER"]
    if not kanten:
        kanten = r.edges()
    return dict(**_ergebnis(fillet(kanten, 0.15)), kanten=len(kanten))


def fall_treppe_wangenkanten_runden():
    import teile
    from build123d import Axis, fillet

    t = teile.treppe()
    # alle geraden Kanten parallel zur X-Achse (Stufen-Vorderkanten)
    kanten = t.edges().filter_by(Axis.X)
    return dict(**_ergebnis(fillet(kanten, 0.2)), kanten=len(kanten))


def fall_rumpf_schale():
    """Freiform-Rumpf (STEP) aushoehlen, Deck offen, Wand 20 mm bei 12 m Laenge."""
    from build123d import Axis, import_brep, offset

    rumpf = import_brep(str(CACHE / "gpph_hull.step.brep"))
    deck = rumpf.faces().sort_by(Axis.Z)[-1]
    return _ergebnis(offset(rumpf, amount=-20, openings=[deck]))


def fall_loft_rumpf_schale():
    """Modell-Rumpf als Loft durch 5 Spantquerschnitte, dann Schale 1,2 mm."""
    from build123d import Axis, Ellipse, Plane, loft, offset

    spanten = []
    for x, (b, t) in zip(
        [0, 60, 120, 180, 240], [(5, 3), (30, 18), (36, 22), (32, 20), (10, 8)], strict=True
    ):
        spanten.append(Plane.YZ.offset(x) * Ellipse(b / 2, t / 2))
    rumpf = loft(spanten)
    oben = rumpf.faces().sort_by(Axis.Z)[-1]
    erg = _ergebnis(offset(rumpf, amount=-1.2, openings=[oben]))
    erg["hinweis"] = "Ellipsen-Loft hat keine ebene Deckflaeche; geoeffnet wird die hoechste Flaeche"
    return erg


def fall_loft_rumpf_halbiert_schale():
    """Wie oben, aber Rumpf am Deck (z=0) eben abgeschnitten: typischer Modellrumpf."""
    from build123d import Align, Axis, Box, Ellipse, Plane, loft, offset

    spanten = []
    for x, (b, t) in zip(
        [0, 60, 120, 180, 240], [(5, 3), (30, 18), (36, 22), (32, 20), (10, 8)], strict=True
    ):
        spanten.append(Plane.YZ.offset(x) * Ellipse(b / 2, t))
    rumpf = loft(spanten) & Box(400, 100, 100, align=(Align.CENTER, Align.CENTER, Align.MAX))
    deck = rumpf.faces().sort_by(Axis.Z)[-1]
    return _ergebnis(offset(rumpf, amount=-1.2, openings=[deck]))


def fall_200_bohrungen():
    """Platte mit 200 Bohrungen als einzelne Bool-Operationen und als eine."""
    from build123d import Box, Cylinder, Pos

    t = time.perf_counter()
    platte = Box(200, 100, 3)
    for i in range(20):
        for j in range(10):
            platte = platte - Pos(-95 + i * 10, -45 + j * 10, 0) * Cylinder(1.5, 5)
    t_einzeln = time.perf_counter() - t
    t = time.perf_counter()
    platte2 = Box(200, 100, 3) - [
        Pos(-95 + i * 10, -45 + j * 10, 0) * Cylinder(1.5, 5) for i in range(20) for j in range(10)
    ]
    t_einmal = time.perf_counter() - t
    import math

    soll = 200 * 100 * 3 - 200 * math.pi * 1.5**2 * 3
    erg = _ergebnis(platte, soll)
    erg.update(einzeln_s=t_einzeln, einmal_s=t_einmal, einmal_volumen=platte2.volume)
    return erg


def fall_zu_grosser_radius():
    """Fehlermeldung bei unmoeglicher Verrundung (Radius > halbe Wandstaerke)."""
    from build123d import Box, fillet

    b = Box(10, 10, 1)
    try:
        r = fillet(b.edges(), 0.8)
        return dict(**_ergebnis(r), hinweis="kein Fehler geworfen")
    except Exception as e:  # noqa: BLE001
        return dict(ok=True, erwarteter_fehler=f"{type(e).__name__}: {str(e)[:300]}")


def fall_rohr_enger_radius():
    """Rohr Ø0,8 mit Biegeradius 0,3 (< Rohrradius): Selbstdurchdringung."""
    from build123d import Circle, FilletPolyline, Plane, Wire, sweep

    pfad = Wire(FilletPolyline((0, 0, 0), (0, 0, 10), (10, 0, 10), radius=0.3).edges())
    prof = (Plane(origin=pfad @ 0, z_dir=pfad % 0) * Circle(0.4)).face()
    r = sweep(prof, path=pfad)
    erg = _ergebnis(r)
    erg["hinweis"] = "Erwartet: ungueltig oder Fehler; ok=True hiesse, der Kern merkt es nicht"
    return erg


FAELLE = {k: v for k, v in globals().items() if k.startswith("fall_")}


def _lauf(name, q):
    sys.path.insert(0, str(HIER))
    t = time.perf_counter()
    try:
        erg = FAELLE[name]()
    except Exception as e:  # noqa: BLE001
        erg = dict(ok=False, fehler=f"{type(e).__name__}: {str(e)[:400]}", spur=traceback.format_exc()[-600:])
    erg["dauer_s"] = time.perf_counter() - t
    q.put(erg)


def main():
    ctx = mp.get_context("spawn")
    ergebnisse = {}
    for name in FAELLE:
        q = ctx.Queue()
        p = ctx.Process(target=_lauf, args=(name, q))
        t = time.perf_counter()
        p.start()
        p.join(GRENZE_S)
        if p.is_alive():
            p.kill()
            p.join()
            erg = dict(ok=False, fehler=f"Zeitgrenze {GRENZE_S} s", dauer_s=time.perf_counter() - t)
        elif q.empty():
            erg = dict(ok=False, fehler=f"Prozess abgestuerzt, Code {p.exitcode}")
        else:
            erg = q.get()
        ergebnisse[name] = erg
        print(name, json.dumps({k: v for k, v in erg.items() if k != "spur"}, ensure_ascii=False))
    OUT.mkdir(exist_ok=True)
    (OUT / "m0_schwaechen.json").write_text(json.dumps(ergebnisse, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
