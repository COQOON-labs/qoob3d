// Every expected value follows from a formula, never from a run of the kernel.

const TOL = 1e-6;   // mm³ and mm: the kernel measures the exact B-Rep

check("block", || {
    let s = part("Block");
    expect_near(s.volume(), 40 * 30 * 10, TOL, "volume = x * y * z");
    expect_near(s.bbox().min[0], -20, TOL, "centred in x");
    expect_near(s.bbox().min[2], 0, TOL, "stands on the bed");
    expect_near(s.bbox().max[2], 10, TOL, "height");
    expect_eq(s.solids(), 1, "one body");
});

check("ball", || {
    let s = part("Ball");
    expect_near(s.volume(), 4.0 / 3.0 * PI() * 10 * 10 * 10, TOL, "volume = 4/3 pi r^3");
    expect_near(s.bbox().min[2], 0, TOL, "rests on the bed");
    expect_near(s.bbox().size[0], 20, TOL, "diameter");
});

check("plate", || {
    let s = part("Plate");
    let w = value("plate.width");
    let d = value("plate.depth");
    let t = value("plate.thickness");
    let r = value("plate.hole") / 2.0;
    expect_near(s.volume(), w * d * t - PI() * r * r * t, TOL, "volume = w d t - pi r^2 t");
    expect_near(s.bbox().size[0], w, TOL, "width");
    expect_eq(s.solids(), 1, "one body");
});

check("peg", || {
    let s = part("Peg");
    expect_near(s.volume(), PI() * 3 * 3 * 8, TOL, "volume = pi r^2 h");
    expect_near(s.bbox().min[2], 10, TOL, "stands on the block");
    expect_near(distance(s, part("Block")), 0, TOL, "touches the block");
});

check("ring", || {
    let s = part("Ring");
    expect_near(s.volume(), PI() * (14 * 14 - 8 * 8) * 6, TOL, "volume = pi (R^2 - r^2) h");
    expect_near(s.bbox().size[0], 28, TOL, "outer diameter");
    expect_near(s.bbox().size[2], 6, TOL, "height");
});

check("angle", || {
    let s = part("Angle");
    expect_near(s.volume(), (30 * 4 + 4 * 16) * 15, TOL, "volume = area of the L * height");
    expect_near(s.bbox().size[1], 20, TOL, "leg length");
});

check("frustum", || {
    let s = part("Frustum");
    // h/3 (A1 + A2 + sqrt(A1 A2)) with A1 = 400, A2 = 100.
    expect_near(s.volume(), 15.0 / 3.0 * (400 + 100 + 200), TOL, "volume of the frustum");
    expect_near(s.bbox().max[2], 15, TOL, "height");
});

check("rounded", || {
    let s = part("Rounded");
    // Each of the four corners loses r^2 - pi r^2 / 4 of the cross-section.
    let area = 20 * 20 - (4 - PI()) * 3 * 3;
    expect_near(s.volume(), area * 10, TOL, "volume = (w^2 - (4 - pi) r^2) h");
    expect_eq(edges(s).circles().len(), 8, "four rounded corners, top and bottom");
});
