//! Indexed mesh → polyhedral B-rep for `import_mesh` (AG-SC-23, ADR-0034).
//! The topology is computed here from the welded indices, OCCT only builds
//! it (`qoob3d_polyhedron`): one vertex per index and one edge per index
//! pair, so neighbouring faces share their edges without sewing.
//!
//! - Every edge must have exactly two triangles running it in opposite
//!   directions; open and non-manifold edges are counted and refused.
//! - Every group of triangles connected through edges is one closed shell.
//!   A shell with positive volume (outward winding) is the outside of a
//!   solid; one with negative volume is a cavity and becomes an inner shell
//!   of the smallest solid that encloses it. A negative shell inside no
//!   solid is inside out and refused.
//! - Adjacent coplanar triangles become one planar face. Its boundary
//!   edges (whose other triangle lies in another face) chain into loops:
//!   the outline runs counter-clockwise about the outward normal, holes
//!   clockwise, so non-convex faces and faces with holes come out right.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use qoob3d_core::import::ImportedMesh;

use crate::occt::ffi::Qoob3dPolyhedron;

/// Sine of the angle below which two triangle normals count as parallel.
const COPLANAR_SIN: f64 = 1e-6;
/// Distance from a face's plane, mm, below which a point lies in it.
const COPLANAR_MM: f64 = 1e-5;
/// Marks a triangle without a face yet.
const NONE: u32 = u32::MAX;

type Point = [f64; 3];

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: Point) -> f64 {
    dot(a, a).sqrt()
}

/// The mesh with its triangles' neighbours: `neighbours[t][k]` lies across
/// the side from corner `k` to corner `k + 1` of triangle `t`.
struct Mesh<'a> {
    positions: &'a [Point],
    triangles: &'a [[u32; 3]],
    neighbours: Vec<[u32; 3]>,
}

impl Mesh<'_> {
    fn corner(&self, t: u32, k: usize) -> Point {
        self.positions[self.triangles[t as usize][k] as usize]
    }

    /// Unit normal of `t`; `None` for a triangle without area (collinear corners).
    fn normal(&self, t: u32) -> Option<Point> {
        let [a, b, c] = [0, 1, 2].map(|k| self.corner(t, k));
        let n = cross(sub(b, a), sub(c, a));
        let scale =
            [sub(b, a), sub(c, b), sub(a, c)].map(|e| dot(e, e)).into_iter().fold(0.0, f64::max);
        let len = length(n);
        (len > 1e-12 * scale).then(|| n.map(|x| x / len))
    }
}

/// Uses of an undirected edge: how many triangles, how many from the lower
/// to the higher index, and the first two triangles with their side.
struct Uses {
    count: u32,
    forward: u32,
    sides: [(u32, usize); 2],
}

/// Neighbours of every triangle; refuses open and non-manifold edges.
fn neighbours<'a>(mesh: &'a ImportedMesh) -> Result<Mesh<'a>, String> {
    let mut edges: HashMap<(u32, u32), Uses> = HashMap::new();
    for (t, tri) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            let side = (t as u32, k);
            let uses = edges.entry((a.min(b), a.max(b))).or_insert(Uses {
                count: 0,
                forward: 0,
                sides: [side; 2],
            });
            if uses.count == 1 {
                uses.sides[1] = side;
            }
            uses.count += 1;
            uses.forward += u32::from(a < b);
        }
    }
    let open = edges.values().filter(|u| u.count == 1).count();
    let bad = edges.values().filter(|u| u.count > 2 || (u.count == 2 && u.forward != 1)).count();
    if open + bad > 0 {
        return Err(format!(
            "the mesh is not a closed surface: {open} open (boundary) edges and {bad} \
             non-manifold or inconsistently oriented edges"
        ));
    }
    let mut neighbours = vec![[NONE; 3]; mesh.triangles.len()];
    for uses in edges.values() {
        let [(t0, k0), (t1, k1)] = uses.sides;
        neighbours[t0 as usize][k0] = t1;
        neighbours[t1 as usize][k1] = t0;
    }
    Ok(Mesh { positions: &mesh.positions, triangles: &mesh.triangles, neighbours })
}

/// Groups of triangles connected through edges, in order of their first triangle.
fn components(mesh: &Mesh<'_>) -> Vec<Vec<u32>> {
    let mut seen = vec![false; mesh.triangles.len()];
    let mut out = Vec::new();
    for seed in 0..mesh.triangles.len() {
        if seen[seed] {
            continue;
        }
        seen[seed] = true;
        let mut group = vec![seed as u32];
        let mut i = 0;
        while let Some(&t) = group.get(i) {
            i += 1;
            for n in mesh.neighbours[t as usize] {
                if !std::mem::replace(&mut seen[n as usize], true) {
                    group.push(n);
                }
            }
        }
        out.push(group);
    }
    out
}

/// A planar face: its triangles and its plane.
struct Face {
    triangles: Vec<u32>,
    origin: Point,
    normal: Point,
}

impl Face {
    fn holds(&self, mesh: &Mesh<'_>, t: u32) -> bool {
        let near = (0..3)
            .all(|k| dot(sub(mesh.corner(t, k), self.origin), self.normal).abs() <= COPLANAR_MM);
        near && mesh.normal(t).is_none_or(|n| {
            dot(n, self.normal) > 0.0 && length(cross(n, self.normal)) <= COPLANAR_SIN
        })
    }
}

/// The planar faces of one component; `face_of` maps triangles to faces,
/// numbered from `first`.
fn faces(
    mesh: &Mesh<'_>,
    component: &[u32],
    first: u32,
    face_of: &mut [u32],
) -> Result<Vec<Face>, String> {
    let mut faces: Vec<Face> = Vec::new();
    for &seed in component {
        let Some(normal) = mesh.normal(seed).filter(|_| face_of[seed as usize] == NONE) else {
            continue;
        };
        let id = first + faces.len() as u32;
        let mut face = Face { triangles: vec![seed], origin: mesh.corner(seed, 0), normal };
        face_of[seed as usize] = id;
        let mut i = 0;
        while let Some(&t) = face.triangles.get(i) {
            i += 1;
            for n in mesh.neighbours[t as usize] {
                if face_of[n as usize] == NONE && face.holds(mesh, n) {
                    face_of[n as usize] = id;
                    face.triangles.push(n);
                }
            }
        }
        faces.push(face);
    }
    // Triangles without area join a neighbouring face that holds them.
    let mut lost: Vec<u32> =
        component.iter().copied().filter(|&t| face_of[t as usize] == NONE).collect();
    while !lost.is_empty() {
        let before = lost.len();
        lost.retain(|&t| {
            let home = mesh.neighbours[t as usize]
                .into_iter()
                .map(|n| face_of[n as usize])
                .find(|&f| f != NONE && faces[(f - first) as usize].holds(mesh, t));
            let Some(f) = home else { return true };
            face_of[t as usize] = f;
            faces[(f - first) as usize].triangles.push(t);
            false
        });
        if lost.len() == before {
            return Err(format!("{before} triangles have no area and lie in no neighbouring face"));
        }
    }
    Ok(faces)
}

/// The best plane through the corners of `face` and how far they lie from it.
fn plane(mesh: &Mesh<'_>, face: &Face) -> (Point, Point, f64) {
    let mut sum = [0.0; 3];
    let mut centre = [0.0; 3];
    for &t in &face.triangles {
        let [a, b, c] = [0, 1, 2].map(|k| mesh.corner(t, k));
        let n = cross(sub(b, a), sub(c, a));
        for k in 0..3 {
            sum[k] += n[k];
            centre[k] += (a[k] + b[k] + c[k]) / 3.0;
        }
    }
    let len = length(sum);
    let normal = if len > 0.0 { sum.map(|x| x / len) } else { face.normal };
    let origin = centre.map(|x| x / face.triangles.len() as f64);
    let off = face
        .triangles
        .iter()
        .flat_map(|&t| (0..3).map(move |k| (t, k)))
        .map(|(t, k)| dot(sub(mesh.corner(t, k), origin), normal).abs())
        .fold(0.0, f64::max);
    (origin, normal, off)
}

/// The boundary loops of `face` (id `id`) as point index chains, the
/// outline first.
fn loops(
    mesh: &Mesh<'_>,
    face_of: &[u32],
    (id, face): (u32, &Face),
    normal: Point,
) -> Result<Vec<Vec<u32>>, String> {
    // Boundary sides a → b, keyed by a; ordered, so the loops and with them
    // the edge order of the B-rep are the same in every run.
    let mut outgoing: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &t in &face.triangles {
        let tri = mesh.triangles[t as usize];
        for k in 0..3 {
            if face_of[mesh.neighbours[t as usize][k] as usize] != id {
                outgoing.entry(tri[k]).or_default().push(tri[(k + 1) % 3]);
            }
        }
    }
    let (u, v) = basis(normal);
    let angle = |from: u32, to: u32| {
        let d = sub(mesh.positions[to as usize], mesh.positions[from as usize]);
        dot(d, v).atan2(dot(d, u))
    };
    let mut chains: Vec<(f64, Vec<u32>)> = Vec::new();
    // A start with one side, where possible: at a point where the face
    // touches itself the way out depends on the way in.
    while let Some(start) = outgoing
        .iter()
        .find(|(_, ends)| ends.len() == 1)
        .or_else(|| outgoing.iter().next())
        .map(|(&a, _)| a)
    {
        let mut chain = vec![start];
        let (mut at, mut previous) = (start, None);
        loop {
            let Some(ends) = outgoing.get_mut(&at) else {
                return Err("a face outline does not close".into());
            };
            // Keep the material on the left: the first side clockwise from
            // the way back.
            let pick = match previous {
                Some(p) if ends.len() > 1 => {
                    let back = angle(at, p);
                    let turn = |e: u32| (back - angle(at, e)).rem_euclid(std::f64::consts::TAU);
                    (0..ends.len())
                        .min_by(|&i, &j| turn(ends[i]).total_cmp(&turn(ends[j])))
                        .unwrap_or(0)
                }
                _ => 0,
            };
            let next = ends.swap_remove(pick);
            if ends.is_empty() {
                outgoing.remove(&at);
            }
            (previous, at) = (Some(at), next);
            if at == start {
                break;
            }
            chain.push(at);
        }
        let points: Vec<Point> = chain.iter().map(|&i| mesh.positions[i as usize]).collect();
        let area: f64 = (1..points.len().saturating_sub(1))
            .map(|i| {
                dot(cross(sub(points[i], points[0]), sub(points[i + 1], points[0])), normal) / 2.0
            })
            .sum();
        chains.push((area, chain));
    }
    let outlines = chains.iter().filter(|(area, _)| *area > 0.0).count();
    if outlines != 1 {
        return Err(format!("a planar face has {outlines} outlines instead of one"));
    }
    chains.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(chains.into_iter().map(|(_, chain)| chain).collect())
}

/// Two unit vectors that span the plane with `normal`.
fn basis(normal: Point) -> (Point, Point) {
    let helper = if normal[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let u = cross(normal, helper);
    let u = u.map(|x| x / length(u));
    (u, cross(normal, u))
}

/// The polyhedral solids of a welded mesh, for `qoob3d_polyhedron`.
///
/// # Errors
/// A geometry message: open or non-manifold edges (with their counts), a
/// group of triangles without positive volume, or a face that has no
/// single outline.
pub(super) fn polyhedron(mesh: &ImportedMesh) -> Result<Qoob3dPolyhedron, String> {
    let topology = neighbours(mesh)?;
    let mut p = Qoob3dPolyhedron {
        points: mesh.positions.iter().flatten().copied().collect(),
        edges: Vec::new(),
        loops: Vec::new(),
        loop_starts: vec![0],
        face_starts: vec![0],
        planes: Vec::new(),
        tolerances: Vec::new(),
        triangles: Vec::new(),
        triangle_starts: vec![0],
        shell_starts: vec![0],
        solid_starts: vec![0],
    };
    let mut edge_index: HashMap<(u32, u32), u32> = HashMap::new();
    let mut face_of = vec![NONE; mesh.triangles.len()];
    let groups = components(&topology);
    for solid in solids(&topology, &groups)? {
        for &shell in &solid {
            add_shell(mesh, &topology, &groups[shell], &mut p, &mut edge_index, &mut face_of)?;
            p.shell_starts.push(p.tolerances.len() as u32);
        }
        p.solid_starts.push(p.shell_starts.len() as u32 - 1);
    }
    Ok(p)
}

/// The shells (indices into `groups`) of every solid, its outside first:
/// each cavity goes to the smallest outside that encloses it.
fn solids(topology: &Mesh<'_>, groups: &[Vec<u32>]) -> Result<Vec<Vec<usize>>, String> {
    let volumes: Vec<f64> = groups.iter().map(|g| signed_volume(topology, g)).collect();
    let mut solids: Vec<Vec<usize>> =
        (0..groups.len()).filter(|&g| volumes[g] > 0.0).map(|g| vec![g]).collect();
    let mut inside_out = 0;
    for cavity in (0..groups.len()).filter(|&g| volumes[g] <= 0.0) {
        let probe = topology.corner(groups[cavity][0], 0);
        let host = solids
            .iter()
            .enumerate()
            .filter(|(_, s)| encloses(topology, &groups[s[0]], probe))
            .min_by(|(_, a), (_, b)| volumes[a[0]].total_cmp(&volumes[b[0]]))
            .map(|(i, _)| i);
        match host {
            Some(i) => solids[i].push(cavity),
            None => inside_out += 1,
        }
    }
    if inside_out > 0 {
        return Err(format!(
            "{inside_out} of {} closed surfaces enclose no positive volume and lie inside no \
             other surface (inside out)",
            groups.len()
        ));
    }
    Ok(solids)
}

/// Whether `point` lies inside the closed surface `group`: a ray in a
/// skew direction (missing edges and vertices of axis-aligned meshes)
/// crosses it an odd number of times.
fn encloses(mesh: &Mesh<'_>, group: &[u32], point: Point) -> bool {
    let dir = {
        let d = [1.0, 0.000_123_7, 0.000_457_1];
        let l = length(d);
        d.map(|c| c / l)
    };
    let crossings = group
        .iter()
        .filter(|&&t| {
            let [a, b, c] = [0, 1, 2].map(|k| mesh.corner(t, k));
            let (e1, e2) = (sub(b, a), sub(c, a));
            let p = cross(dir, e2);
            let det = dot(e1, p);
            if det.abs() < 1e-15 {
                return false;
            }
            let s = sub(point, a);
            let u = dot(s, p) / det;
            let q = cross(s, e1);
            let v = dot(dir, q) / det;
            u >= 0.0 && v >= 0.0 && u + v <= 1.0 && dot(e2, q) / det > 0.0
        })
        .count();
    crossings % 2 == 1
}

/// The faces of one closed shell, appended to `p`.
fn add_shell(
    mesh: &ImportedMesh,
    topology: &Mesh<'_>,
    group: &[u32],
    p: &mut Qoob3dPolyhedron,
    edge_index: &mut HashMap<(u32, u32), u32>,
    face_of: &mut [u32],
) -> Result<(), String> {
    {
        let first = p.tolerances.len() as u32;
        // Face ids are global, so `face_of` tells the faces of all groups apart.
        let faces = faces(topology, group, first, face_of)?;
        for (id, face) in (first..).zip(&faces) {
            let (origin, normal, off) = plane(topology, face);
            for chain in loops(topology, face_of, (id, face), normal)? {
                for (i, &a) in chain.iter().enumerate() {
                    let b = chain[(i + 1) % chain.len()];
                    let next = edge_index.len() as u32;
                    let edge = match edge_index.entry((a.min(b), a.max(b))) {
                        Entry::Occupied(e) => *e.get(),
                        Entry::Vacant(e) => {
                            p.edges.extend([a.min(b), a.max(b)]);
                            *e.insert(next)
                        }
                    };
                    p.loops.push(2 * edge + u32::from(a > b));
                }
                p.loop_starts.push(p.loops.len() as u32);
            }
            p.face_starts.push(p.loop_starts.len() as u32 - 1);
            p.planes.extend(origin.into_iter().chain(normal));
            // Margin over the largest distance, so OCCT's own check agrees.
            p.tolerances.push(2.0 * off);
            p.triangles.extend(face.triangles.iter().flat_map(|&t| mesh.triangles[t as usize]));
            p.triangle_starts.push((p.triangles.len() / 3) as u32);
        }
    }
    Ok(())
}

fn signed_volume(mesh: &Mesh<'_>, group: &[u32]) -> f64 {
    group
        .iter()
        .map(|&t| {
            let [a, b, c] = [0, 1, 2].map(|k| mesh.corner(t, k));
            dot(a, cross(b, c)) / 6.0
        })
        .sum()
}
