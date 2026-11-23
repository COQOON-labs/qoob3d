// Solids from profiles and an edge treatment.
// The expected volumes are in checks/volumes.rhai.

// Half section: x is the radius, y the height.
part("Ring", #{ material: "PLA" }, |deps| {
    revolve(polygon([[8, 0], [14, 0], [14, 6], [8, 6]]), 360)
});

part("Angle", #{ material: "PLA" }, |deps| {
    extrude(polygon([[0, 0], [30, 0], [30, 4], [4, 4], [4, 20], [0, 20]]), 15)
});

part("Frustum", #{ material: "PLA" }, |deps| {
    loft([rect(20, 20).at_z(0), rect(10, 10).at_z(15)])
});

part("Rounded", #{ material: "PLA" }, |deps| {
    let b = box(20, 20, 10);
    fillet(b, edges(b).parallel("z"), 3)
});
