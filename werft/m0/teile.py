"""M0-Probeteile (Konzept Abschnitt 8) in build123d.

Jedes Teil ist eine reine Funktion ohne Argumente und liefert einen Koerper.
Jede Konstante ist entweder aus Konzept Abschnitt 10 (QUELLE) oder eine
Annahme (ANNAHME), weil Abschnitt 10 das Mass nicht nennt.

Zu jedem Teil gibt es eine Funktion ``*_soll_volumen`` mit dem analytisch
berechneten Volumen derselben Geometrie. Sie dient nur der Pruefung, ob der
Kern richtig rechnet; gebaut wird ausschliesslich mit build123d/OCCT.
"""

from __future__ import annotations

import math

from build123d import (
    Align,
    Axis,
    Box,
    Compound,
    Cylinder,
    FilletPolyline,
    Line,
    Location,
    Plane,
    Polyline,
    Pos,
    Rot,
    Wire,
    Circle,
    make_face,
    revolve,
    sweep,
)

# ---------------------------------------------------------------------------
# Kransaeule: Drehkoerper mit Fasen, Zapfen unten, Tasche oben
# ---------------------------------------------------------------------------
KS_D = 13.0  # QUELLE Abschnitt 10: Saeule Ø 13 mm
KS_H = 20.0  # ANNAHME: Hoehe der Saeule
KS_FASE = 0.5  # ANNAHME: Fase an Ober- und Unterkante der Saeule
KS_ZAPFEN_D = 4.0  # QUELLE Abschnitt 10: Zapfen Ø 4 mm unten
KS_ZAPFEN_L = 2.45  # QUELLE Abschnitt 5 (Beispielcode): zapfen(d=4, l=2.45)
KS_ZAPFEN_FASE = 0.3  # ANNAHME: Einfuehrfase am Zapfenende
KS_TASCHE = (4.4, 5.4, 4.2)  # QUELLE Abschnitt 10: Tasche 4,4 x 5,4 x 4,2 mm oben


def kransaeule():
    r, h, c = KS_D / 2, KS_H, KS_FASE
    rz, lz, cz = KS_ZAPFEN_D / 2, KS_ZAPFEN_L, KS_ZAPFEN_FASE
    # Halbprofil in der XZ-Ebene (x = Radius), um Z gedreht.
    profil = Polyline(
        (0, -lz),
        (rz - cz, -lz),
        (rz, -lz + cz),
        (rz, 0),
        (r - c, 0),
        (r, c),
        (r, h - c),
        (r - c, h),
        (0, h),
        close=True,
    )
    koerper = revolve(make_face(Plane.XZ * profil), Axis.Z, 360)
    tx, ty, tz = KS_TASCHE
    tasche = Pos(0, 0, h) * Box(tx, ty, tz, align=(Align.CENTER, Align.CENTER, Align.MAX))
    return koerper - tasche


def fasenring(radius: float, fase: float) -> float:
    """Volumen eines um die Achse laufenden Dreiecks (Pappus)."""
    return 2 * math.pi * (radius - fase / 3) * fase**2 / 2


def kransaeule_soll_volumen() -> float:
    r = KS_D / 2
    rz = KS_ZAPFEN_D / 2
    v = math.pi * r**2 * KS_H - 2 * fasenring(r, KS_FASE)
    v += math.pi * rz**2 * KS_ZAPFEN_L - fasenring(rz, KS_ZAPFEN_FASE)
    v -= math.prod(KS_TASCHE)
    return v


# ---------------------------------------------------------------------------
# Treppe: zwei Wangen, 11 Stufen mit je 2 Schlitzen, 6 Pfostenaugen
# ---------------------------------------------------------------------------
TR_N = 11  # QUELLE: 11 Steigungen / 11 Stufen
TR_STEIGUNG = 5.91  # QUELLE: 5,91 mm
TR_WINKEL = 52.4  # QUELLE: Neigung 52,4°
TR_BREITE = 20.0  # QUELLE: aussen 20 mm breit
TR_HOEHE = 65.0  # QUELLE: Hoehe 65 mm
TR_WANGE_T = 1.6  # QUELLE: Wange 1,6 x 5 mm (1,6 = Dicke)
TR_WANGE_B = 5.0  # QUELLE: Wange 1,6 x 5 mm (5 = Breite quer zur Neigung, ANNAHME der Lesart)
TR_STUFE_TIEFE = 5.0  # ANNAHME
TR_STUFE_DICKE = 0.8  # ANNAHME
TR_SCHLITZ_B = 0.6  # QUELLE: Schlitze 0,6 mm
TR_SCHLITZ_L = 12.0  # ANNAHME: Schlitzlaenge
TR_SCHLITZ_ABSTAND = 2.0  # ANNAHME: Mittenabstand der beiden Schlitze in Stufentiefe
TR_AUGE_D = 2.2  # ANNAHME: Aussendurchmesser Pfostenauge
TR_AUGE_H = 2.0  # ANNAHME: Hoehe Pfostenauge
TR_AUGE_BOHRUNG = 0.9  # QUELLE: Bohrung Ø 0,9
TR_AUGE_UEBERLAPPUNG = 0.5  # ANNAHME: Auge greift 0,5 mm in die Wange
TR_AUGEN_STUFEN = (2, 6, 10)  # ANNAHME: Augen an diesen Stufen, je Seite 3 = 6 Augen


def _treppe_lauf() -> float:
    return TR_STEIGUNG / math.tan(math.radians(TR_WINKEL))


def _wange(x0: float):
    """Wange als schraeger Streifen, unten bei z=0 und oben bei z=H abgeschnitten.

    Seitenansicht in der YZ-Ebene: Mittellinie z = y * tan(winkel).
    """
    w = math.radians(TR_WINKEL)
    vorlauf = 2 * TR_WANGE_B  # ueber z=0 und z=H hinaus, wird beschnitten
    lang = TR_HOEHE / math.sin(w) + 2 * vorlauf
    streifen = (
        Pos(x0, 0, 0)
        * Rot(TR_WINKEL, 0, 0)
        * Pos(0, -vorlauf, 0)
        * Box(TR_WANGE_T, lang, TR_WANGE_B, align=(Align.MIN, Align.MIN, Align.CENTER))
    )
    gross = 10 * TR_HOEHE
    schnitt = Pos(0, 0, TR_HOEHE / 2) * Box(gross, gross, TR_HOEHE)
    return streifen & schnitt


def _stufe(i: int):
    """Stufe i (1..N): Oberkante auf z = i*Steigung, mittig im Wangenstreifen."""
    innen = TR_BREITE - 2 * TR_WANGE_T
    z = i * TR_STEIGUNG
    y = z / math.tan(math.radians(TR_WINKEL))  # Mittellinie der Wange auf dieser Hoehe
    stufe = Pos(0, y, z) * Box(
        innen, TR_STUFE_TIEFE, TR_STUFE_DICKE, align=(Align.CENTER, Align.CENTER, Align.MAX)
    )
    for dy in (-TR_SCHLITZ_ABSTAND / 2, TR_SCHLITZ_ABSTAND / 2):
        stufe -= Pos(0, y + dy, z) * Box(
            TR_SCHLITZ_L, TR_SCHLITZ_B, 3 * TR_STUFE_DICKE, align=(Align.CENTER, Align.CENTER, Align.MAX)
        )
    return stufe


def _auge(seite: int, i: int):
    r = TR_AUGE_D / 2
    x = seite * (TR_BREITE / 2 + r - TR_AUGE_UEBERLAPPUNG)
    z = i * TR_STEIGUNG
    y = z / math.tan(math.radians(TR_WINKEL))
    auge = Pos(x, y, z) * Cylinder(r, TR_AUGE_H)
    return auge - Pos(x, y, z) * Cylinder(TR_AUGE_BOHRUNG / 2, 2 * TR_AUGE_H)


def treppe():
    teil = _wange(-TR_BREITE / 2) + _wange(TR_BREITE / 2 - TR_WANGE_T)
    for i in range(1, TR_N + 1):
        teil += _stufe(i)
    for seite in (-1, 1):
        for i in TR_AUGEN_STUFEN:
            teil += _auge(seite, i)
    return teil


def treppe_soll_volumen() -> float:
    w = math.radians(TR_WINKEL)
    wangen = 2 * TR_WANGE_T * TR_HOEHE * TR_WANGE_B / math.sin(w)
    innen = TR_BREITE - 2 * TR_WANGE_T
    stufe = innen * TR_STUFE_TIEFE * TR_STUFE_DICKE - 2 * TR_SCHLITZ_L * TR_SCHLITZ_B * TR_STUFE_DICKE
    r = TR_AUGE_D / 2
    d = r - TR_AUGE_UEBERLAPPUNG  # Abstand Augenmitte - Wangenflaeche
    segment = r**2 * math.acos(d / r) - d * math.sqrt(r**2 - d**2)
    auge = (math.pi * r**2 - segment - math.pi * (TR_AUGE_BOHRUNG / 2) ** 2) * TR_AUGE_H
    return wangen + TR_N * stufe + 2 * len(TR_AUGEN_STUFEN) * auge


TR_FUSION_VOLUMEN = 2428.5  # QUELLE Abschnitt 10: Druckteil 2,4285 cm³


# ---------------------------------------------------------------------------
# Relingabschnitt aus Draht
# ---------------------------------------------------------------------------
RL_D = 0.8  # QUELLE: Pfosten und Handlauf Ø 0,8
RL_D_DURCHZUG = 0.5  # QUELLE: Durchzuege Ø 0,5
RL_DURCHZUG_Z = (8.0, 15.0)  # QUELLE: auf +8 und +15
RL_HANDLAUF_Z = 22.0  # QUELLE: Handlauf auf +22
RL_TEILUNG = 24.0  # QUELLE: Pfostenteilung ~24 mm
RL_BIEGERADIUS = 1.5  # QUELLE: Biegeradius >= 1,5 mm
RL_FELDER = 4  # ANNAHME: Abschnitt mit 4 Feldern = 5 Pfosten
RL_FUSS = 2.0  # ANNAHME: Pfosten stecken 2 mm im Deck (z bis -2)


def _rohr(pfad: Wire, d: float):
    profil = (Plane(origin=pfad @ 0, z_dir=pfad % 0) * Circle(d / 2)).face()
    return sweep(profil, path=pfad)


def _handlauf_pfad() -> Wire:
    """U-Form: Endpfosten hoch, Bogen, Handlauf, Bogen, Endpfosten runter (in XZ).

    Die Bogen rundet der Kern (FilletPolyline), nicht eigene Mathematik.
    """
    lang = RL_FELDER * RL_TEILUNG
    z0, zh = -RL_FUSS, RL_HANDLAUF_Z
    return Wire(
        FilletPolyline((0, 0, z0), (0, 0, zh), (lang, 0, zh), (lang, 0, z0), radius=RL_BIEGERADIUS).edges()
    )


def reling_drahtkoerper() -> list:
    """Die einzelnen Drahtkoerper (wie in Fusion getrennte Koerper)."""
    lang = RL_FELDER * RL_TEILUNG
    koerper = [_rohr(_handlauf_pfad(), RL_D)]
    for k in range(1, RL_FELDER):
        pfad = Wire([Line((k * RL_TEILUNG, 0, -RL_FUSS), (k * RL_TEILUNG, 0, RL_HANDLAUF_Z))])
        koerper.append(_rohr(pfad, RL_D))
    for z in RL_DURCHZUG_Z:
        koerper.append(_rohr(Wire([Line((0, 0, z), (lang, 0, z))]), RL_D_DURCHZUG))
    return koerper


def reling():
    """Alle Drahtkoerper als Baugruppe (nicht verschmolzen)."""
    return Compound(reling_drahtkoerper())


def reling_verschmolzen():
    """Druckfassung: alle Draehte zu einem Koerper vereinigt (Bool-Test fuer duenne Rohre)."""
    teile = reling_drahtkoerper()
    ergebnis = teile[0]
    for t in teile[1:]:
        ergebnis = ergebnis + t
    return ergebnis


def reling_pfadlaengen() -> tuple[float, float]:
    """Drahtlaengen (Ø0,8 ; Ø0,5) analytisch."""
    lang = RL_FELDER * RL_TEILUNG
    rb = RL_BIEGERADIUS
    beine = 2 * (RL_HANDLAUF_Z - rb + RL_FUSS)
    handlauf = beine + 2 * (math.pi / 2 * rb) + (lang - 2 * rb)
    pfosten = (RL_FELDER - 1) * (RL_HANDLAUF_Z + RL_FUSS)
    return handlauf + pfosten, len(RL_DURCHZUG_Z) * lang


def reling_soll_volumen() -> float:
    """Summe der Einzelkoerper: Kreisflaeche x Pfadlaenge (Pappus/Guldin, exakt fuer Rohre)."""
    l08, l05 = reling_pfadlaengen()
    return math.pi / 4 * (RL_D**2 * l08 + RL_D_DURCHZUG**2 * l05)


TEILE = {
    "kransaeule": (kransaeule, kransaeule_soll_volumen),
    "treppe": (treppe, treppe_soll_volumen),
    "reling": (reling, reling_soll_volumen),
    "reling_verschmolzen": (reling_verschmolzen, None),
}

__all__ = ["TEILE", "Location"]
