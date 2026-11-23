// Primitives, a boolean with target values and a part that depends on another.
// The expected volumes are in checks/volumes.rhai.

part("Block", #{ material: "PLA" }, |deps| box(40, 30, 10));

// Resting on the bed: the centre is one radius up.
part("Ball", #{ material: "PLA" }, |deps| sphere(20).move(0, 0, 10));

part("Plate", #{ material: "PLA" }, |deps| {
    let t = value("plate.thickness");
    box(value("plate.width"), value("plate.depth"), t) - cylinder(value("plate.hole"), t)
});

// Stands on the block, wherever its top face is.
part("Peg", #{ depends: ["Block"] }, |deps| {
    let top = deps["Block"].bbox().max[2];
    cylinder(6, 8).move(0, 0, top)
});
