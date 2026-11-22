"""M0-Abnahme messen (Konzept Abschnitt 8).

Jede Messung laeuft in einem frischen Arbeitsprozess (spawn) mit harter
Zeitgrenze; der Hauptprozess importiert build123d nie. Ergebnis als JSON nach
out/m0_messung.json und als Tabelle auf stdout.

    uv run python m0/messen.py [--wdh 5]
"""

from __future__ import annotations

import argparse
import json
import multiprocessing as mp
import platform
import statistics
import sys
import time
from pathlib import Path

HIER = Path(__file__).resolve().parent
OUT = HIER.parent / "out"
STEP_DIR = HIER / "step_dl"
CACHE = HIER.parent / ".werft" / "cache"

GRENZE_S = 120  # Zeitgrenze je Arbeitsprozess


# ---------------------------------------------------------------------------
# Arbeitsprozess-Funktionen (laufen im Kindprozess)
# ---------------------------------------------------------------------------
def _job_teil(name: str, q):
    t0 = time.perf_counter()
    sys.path.insert(0, str(HIER))
    import teile  # noqa: E402  (importiert build123d)

    t_import = time.perf_counter() - t0
    bauen, soll = teile.TEILE[name]
    t1 = time.perf_counter()
    teil = bauen()
    t_bau_kalt = time.perf_counter() - t1
    t2 = time.perf_counter()
    teil = bauen()
    t_bau_warm = time.perf_counter() - t2
    t3 = time.perf_counter()
    gueltig = teil.is_valid
    vol = teil.volume
    t_pruef = time.perf_counter() - t3
    erg = dict(
        teil=name,
        import_s=t_import,
        bau_kalt_s=t_bau_kalt,
        bau_warm_s=t_bau_warm,
        pruef_s=t_pruef,
        volumen=vol,
        soll=soll() if soll else None,
        gueltig=bool(gueltig),
        koerper=len(teil.solids()),
        flaechen=len(teil.faces()),
    )
    if name == "treppe":
        erg["fusion_ref"] = teile.TR_FUSION_VOLUMEN
    q.put(erg)


def _job_export(name: str, q):
    sys.path.insert(0, str(HIER))
    import teile
    from build123d import Mesher, export_step, export_stl

    teil = teile.TEILE[name][0]()
    OUT.mkdir(exist_ok=True)
    zeiten = {}
    t = time.perf_counter()
    export_step(teil, str(OUT / f"{name}.step"))
    zeiten["step_s"] = time.perf_counter() - t
    t = time.perf_counter()
    export_stl(teil, str(OUT / f"{name}.stl"), tolerance=0.01, angular_tolerance=0.1)
    zeiten["stl_s"] = time.perf_counter() - t
    t = time.perf_counter()
    m = Mesher()
    m.add_shape(teil, linear_deflection=0.01, angular_deflection=0.1)
    m.write(str(OUT / f"{name}.3mf"))
    zeiten["3mf_s"] = time.perf_counter() - t
    import trimesh

    mesh = trimesh.load(str(OUT / f"{name}.stl"))
    q.put(
        dict(
            teil=name,
            **zeiten,
            stl_dicht=bool(mesh.is_watertight),
            stl_dreiecke=len(mesh.faces),
            stl_volumen=float(mesh.volume),
        )
    )


def _job_step_import(datei: str, q):
    t0 = time.perf_counter()
    from build123d import export_brep, import_step

    t_import = time.perf_counter() - t0
    t1 = time.perf_counter()
    form = import_step(str(STEP_DIR / datei))
    t_step = time.perf_counter() - t1
    CACHE.mkdir(parents=True, exist_ok=True)
    t2 = time.perf_counter()
    export_brep(form, str(CACHE / f"{datei}.brep"))
    t_brep_schreiben = time.perf_counter() - t2
    q.put(
        dict(
            datei=datei,
            groesse_mb=(STEP_DIR / datei).stat().st_size / 1e6,
            import_build123d_s=t_import,
            step_lesen_s=t_step,
            brep_schreiben_s=t_brep_schreiben,
            flaechen=len(form.faces()),
            koerper=len(form.solids()),
            volumen=form.volume,
            gueltig=bool(form.is_valid),
            bbox=[round(v, 2) for v in form.bounding_box().size],
        )
    )


def _job_cache_lesen(datei: str, q):
    t0 = time.perf_counter()
    from build123d import import_brep

    t_import = time.perf_counter() - t0
    t1 = time.perf_counter()
    form = import_brep(str(CACHE / f"{datei}.brep"))
    t_brep = time.perf_counter() - t1
    q.put(
        dict(
            datei=datei,
            import_build123d_s=t_import,
            brep_lesen_s=t_brep,
            flaechen=len(form.faces()),
            volumen=form.volume,
        )
    )


def _job_step_messen(datei: str, q):
    """Abstand, Durchdringung, Strahl an einem importierten Koerper (aus dem Cache)."""
    from build123d import Axis, Box, Pos, Vector, import_brep

    form = import_brep(str(CACHE / f"{datei}.brep"))
    bb = form.bounding_box()
    groesse = max(bb.size)
    erg = dict(datei=datei)

    # Abstand: Quader 5 % der Groesse neben dem Koerper (Luecke = 2 % der Groesse in +X)
    luecke = 0.02 * groesse
    k = 0.05 * groesse
    nachbar = Pos(bb.max.X + luecke + k / 2, bb.center().Y, bb.center().Z) * Box(k, k, k)
    t = time.perf_counter()
    abstand, p1, p2 = form.distance_to_with_closest_points(nachbar)
    erg["abstand_s"] = time.perf_counter() - t
    erg["abstand"] = abstand
    erg["abstand_punkte"] = [list(map(lambda v: round(v, 4), p)) for p in (p1, p2)]
    erg["abstand_bbox_untergrenze"] = luecke  # wahrer Abstand >= Luecke

    # Strahl: senkrecht von oben durch die Mitte der Huelle; erster Treffer
    # auf einer Flaeche. (Solid & Axis liefert die Strecke im Koerper als Edge,
    # daher Flaechen einzeln schneiden.)
    start = Vector(bb.center().X, bb.center().Y, bb.max.Z + 10)
    richtung = Vector(0, 0, -1)
    strahl = Axis(start, richtung)
    t = time.perf_counter()
    flaechen = form.faces_intersected_by_axis(strahl)
    punkte = []
    for fl in flaechen:
        for v in fl.intersect(strahl) or []:
            p = Vector(v) if not hasattr(v, "center") else v.center()
            punkte.append(((p - start).dot(richtung), p))
    punkte.sort(key=lambda e: e[0])
    erg["strahl_s"] = time.perf_counter() - t
    erg["strahl_flaechen"] = len(flaechen)
    erg["strahl_treffer"] = [[round(p.X, 4), round(p.Y, 4), round(p.Z, 4)] for _, p in punkte][:6]

    # Durchdringung: Quader (10 % der Groesse) um den ersten Strahltreffer,
    # ragt also sicher in den Koerper hinein.
    k2 = 0.1 * groesse
    c = punkte[0][1] if punkte else form.center()
    stoerer = Pos(c.X, c.Y, c.Z) * Box(k2, k2, k2)
    t = time.perf_counter()
    schnitt = form & stoerer
    erg["durchdringung_s"] = time.perf_counter() - t
    erg["durchdringung_volumen"] = schnitt.volume if schnitt is not None else 0.0
    erg["durchdringung_gueltig"] = bool(schnitt.is_valid) if schnitt is not None else None
    erg["stoerer_volumen"] = k2**3
    q.put(erg)


# ---------------------------------------------------------------------------
# Hauptprozess: nur Prozessverwaltung, keine Geometrie
# ---------------------------------------------------------------------------
def im_prozess(fn, arg, grenze_s: float = GRENZE_S) -> dict:
    ctx = mp.get_context("spawn")
    q = ctx.Queue()
    t0 = time.perf_counter()
    p = ctx.Process(target=fn, args=(arg, q))
    p.start()
    p.join(grenze_s)
    gesamt = time.perf_counter() - t0
    if p.is_alive():
        p.kill()
        p.join()
        return dict(fehler=f"Zeitgrenze {grenze_s} s ueberschritten", prozess_s=gesamt)
    if q.empty():
        return dict(fehler=f"Prozess beendet mit Code {p.exitcode}", prozess_s=gesamt)
    erg = q.get()
    erg["prozess_s"] = gesamt
    return erg


def zusammenfassen(laeufe: list[dict], felder: list[str]) -> dict:
    erg = dict(laeufe[0])
    for f in felder:
        werte = [l_[f] for l_ in laeufe if f in l_]
        if werte:
            erg[f] = dict(median=statistics.median(werte), max=max(werte), min=min(werte), n=len(werte))
    return erg


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--wdh", type=int, default=5)
    ap.add_argument("--step", nargs="*", default=["nist_ctc_02_asme1_rc.stp", "gpph_hull.step", "synth_10x_ctc02.step"])
    args = ap.parse_args()

    ergebnis = dict(
        rechner=dict(
            system=platform.platform(),
            maschine=platform.machine(),
            python=platform.python_version(),
            kerne=mp.cpu_count(),
        ),
        teile=[],
        exporte=[],
        step=[],
    )
    for name in ["kransaeule", "treppe", "reling", "reling_verschmolzen"]:
        laeufe = [im_prozess(_job_teil, name) for _ in range(args.wdh)]
        fehler = [l_ for l_ in laeufe if "fehler" in l_]
        if fehler:
            ergebnis["teile"].append(dict(teil=name, fehler=fehler[0]["fehler"]))
            continue
        ergebnis["teile"].append(
            zusammenfassen(laeufe, ["import_s", "bau_kalt_s", "bau_warm_s", "pruef_s", "prozess_s"])
        )
        ergebnis["exporte"].append(im_prozess(_job_export, name))

    for datei in args.step:
        imp = [im_prozess(_job_step_import, datei) for _ in range(args.wdh)]
        cache = [im_prozess(_job_cache_lesen, datei) for _ in range(args.wdh)]
        mess = [im_prozess(_job_step_messen, datei) for _ in range(args.wdh)]
        ergebnis["step"].append(
            dict(
                importieren=zusammenfassen(imp, ["step_lesen_s", "brep_schreiben_s", "prozess_s"]),
                cache=zusammenfassen(cache, ["brep_lesen_s", "import_build123d_s", "prozess_s"]),
                messen=zusammenfassen(
                    mess, ["abstand_s", "durchdringung_s", "strahl_s", "prozess_s"]
                ),
            )
        )

    OUT.mkdir(exist_ok=True)
    (OUT / "m0_messung.json").write_text(json.dumps(ergebnis, indent=2, ensure_ascii=False))
    print(json.dumps(ergebnis, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
