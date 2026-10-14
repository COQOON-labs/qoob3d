---
id: ADR-0031
title: Körper-Features (Extrusion, Drehung, Erhebung, Ebene) mit Rebuild über Hash-Kette und Booleans gegen berührende Körper
status: akzeptiert
date: 2026-10-14
crates: [qoob3d-core, qoob3d-kernel]
tags: [dokumentmodell, kernel, features]
supersedes: []
superseded_by: null
---

## Kontext

Nach den Skizzen (M3a) sollen echte druckbare Teile entstehen: Quader, Zylinder, Kugel,
Pyramide. Dafür braucht es Features, die Profile in Körper verwandeln, ein Verhalten für
mehrere Körper (Neu, Verbinden, Ausschneiden) und einen Rebuild, der nur Geändertes neu
rechnet (ADR-0012).

## Entscheidung

- **Formatversion 2** (`document::FORMAT_VERSION`): `FeatureKind` bekommt `Plane`
  (versetzte Konstruktionsebene), `Extrude`, `Revolve`, `Loft`, `SpurGear`, `Coil`
  (ADR-0032). Skizzen bekommen Bögen (`CurveKind::Arc`, Mittelpunkt/Start/Ende, gegen den
  Uhrzeigersinn). `PlaneRef::Construction(FeatureId)` erlaubt Skizzen auf
  Konstruktionsebenen; aufgelöst wird über `Document::plane_frame`. v1 → v2 ist eine leere
  Migration (nur neue Varianten).
- Referenzen: Profile über `ProfileRef { sketch, key }` (ADR-0011), Achsen als Ursprungsachse
  oder Skizzenlinie, Loft-Spitzen als Skizzenpunkt. `validate` verlangt, dass jede Referenz
  auf ein **früheres** Feature zeigt.
- **Operation** je Feature: `NewBody` (jedes Solid ein eigener Körper), `Join` (vereinigt
  mit allen Körpern, die das Ergebnis berührt; keiner → neuer Körper), `Cut` (zieht von allen
  berührten Körpern ab; keiner → Feature-Fehler). „Berührt“ = Abstand ≤ 1e-6 mm.
- Körper-ID = erzeugendes Feature + Index (`BodyId`), Name „Body N“ nach Erzeugungsreihenfolge.
- **Rebuild** im Worker (`model.rs`): Kette `key_i = hash(key_{i-1}, Feature-JSON)`; der
  Zustand (Körper als OCCT-Shapes) nach jedem Feature liegt im Cache. Behalten werden die
  Schlüssel des aktuellen und des vorigen Rebuilds (billiges Undo/Redo). Profile werden je
  Skizze über ihren Hash gecacht, Netze pro Körper beim ersten Bedarf.
- Fehler sind **Feature-Fehler**: Das Feature wird übersprungen, die Körper bleiben wie
  davor, der Grund steht rot in der Timeline. Fehlt ein Profil, wird nicht geraten.
- Booleans werden mit `ShapeUpgrade_UnifySameDomain` vereinfacht (keine Nähte auf ebenen
  Flächen). Entartete Dreiecke (z. B. an Kugelpolen) verwirft der Mesher.

## Konsequenzen

- Unveränderte Features vor einer Änderung kosten beim Rebuild nichts.
- Join/Cut wirken ohne Körperauswahl wie Fusions Voreinstellung („alle berührten Körper“).
  Eine explizite Teilnehmer-Auswahl kann später als Feld dazukommen.
- Die Timeline kann noch nicht zurückgerollt werden (M5).

## Verworfene Alternativen

- **Ein Körper pro Dokument** – mehrteilige Drucke (Platte + Zahnrad) wären nicht möglich.
- **Join/Cut nur gegen den letzten Körper** – überraschend, sobald es mehrere Körper gibt.

## Verweise

- Code: `crates/qoob3d-core/src/document/{feature,mod}.rs`, `crates/qoob3d-kernel/src/model.rs`,
  `crates/qoob3d-kernel/cpp/qoob3d_occt.cpp`
- Tests: `crates/qoob3d-kernel/tests/features.rs`
- Verwandt: ADR-0011, ADR-0012, ADR-0030, ADR-0032, ADR-0033
