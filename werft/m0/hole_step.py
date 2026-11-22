"""Laedt die Ersatz-STEP-Koerper fuer M0 nach m0/step_dl/ (nicht in Git).

Ersatz fuer den noch fehlenden STEP-Export des Schiffsrumpfs:
- nist_ctc_02_asme1_rc.stp: NIST CAD Test Case 02 (oeffentlich, US-Regierung),
  1,15 MB, 664 Flaechen, davon 94 B-Spline.
- gpph_hull.step: Schiffsrumpf mit Freiformflaechen aus crayon-party/Hull-GNN,
  17 B-Spline-Flaechen, 12 m lang.
- synth_10x_ctc02.step: SYNTHETISCH, zehn verschobene Kopien von CTC 02 in
  einer Datei (Lasttest fuer grosse Dateien, ~11 MB).

    uv run python m0/hole_step.py
"""

from __future__ import annotations

import hashlib
import multiprocessing as mp
import urllib.request
from pathlib import Path

ZIEL = Path(__file__).resolve().parent / "step_dl"

QUELLEN = {
    "nist_ctc_02_asme1_rc.stp": "https://raw.githubusercontent.com/BRL-CAD/models/HEAD/NIST/"
    "NIST-PMI-STEP-Files/AP203%20geometry%20only/nist_ctc_02_asme1_rc.stp",
    "gpph_hull.step": "https://raw.githubusercontent.com/crayon-party/Hull-GNN/HEAD/GPPH-A1-B3-Hull.step",
}


def _synth(q):
    from build123d import Compound, Pos, export_step, import_step

    basis = import_step(str(ZIEL / "nist_ctc_02_asme1_rc.stp"))
    kopien = [Pos(i * 800, 0, 0) * basis for i in range(10)]
    export_step(Compound(kopien), str(ZIEL / "synth_10x_ctc02.step"))
    q.put(True)


def main():
    ZIEL.mkdir(exist_ok=True)
    for name, url in QUELLEN.items():
        pfad = ZIEL / name
        if not pfad.exists():
            urllib.request.urlretrieve(url, pfad)
        print(name, pfad.stat().st_size, hashlib.sha256(pfad.read_bytes()).hexdigest()[:16])
    if not (ZIEL / "synth_10x_ctc02.step").exists():
        ctx = mp.get_context("spawn")
        q = ctx.Queue()
        p = ctx.Process(target=_synth, args=(q,))
        p.start()
        p.join(300)
        if p.is_alive():
            p.kill()
            raise SystemExit("Zeitgrenze beim Erzeugen der synthetischen Datei")
    print("synth_10x_ctc02.step", (ZIEL / "synth_10x_ctc02.step").stat().st_size)


if __name__ == "__main__":
    main()
