// Crane column (M0 probe part, werft/m0/teile.py `kransaeule`): a turned
// column with chamfered top and bottom edges, a chamfered pin below and a
// rectangular pocket on top. Units mm, +Z up.
//
// Built from primitives and edge features (AG-SC-04, AG-SC-09, AG-SC-13).
// The same part as a revolved profile is in `crane_column_revolve.rhai`.

let d = 13.0;            // column diameter (source: concept section 10)
let h = 20.0;            // column height (assumption)
let c = 0.5;             // chamfer at the top and bottom edge (assumption)
let pin_d = 4.0;         // pin diameter (source: concept section 10)
let pin_l = 2.45;        // pin length (source: concept section 5)
let pin_c = 0.3;         // lead-in chamfer at the pin end (assumption)
let pocket = [4.4, 5.4, 4.2]; // pocket on top (source: concept section 10)

let column = cylinder(d, h);
column = chamfer(column, edges(column).circles(), c);

let p = pin(pin_d, pin_l);            // from z = 0 downward
p = chamfer(p, edges(p).bottom(), pin_c);

let body = column + p;
let cavity = box(pocket[0], pocket[1], pocket[2]).move(0.0, 0.0, h - pocket[2]);

show(body - cavity, "crane_column");
