//! 3MF reader (AG-SC-23): the core specification plus the production
//! extension as Bambu Studio writes it (`p:path` components in further model
//! files of the archive). Every build item is expanded through its
//! components, nested to any depth, with the 3MF row-vector transforms
//! (`x' = x·m00 + y·m10 + z·m20 + m30`, …) and the model's unit.
//!
//! Limits against hostile archives: the declared sizes of all entries
//! together at most 512 MiB, each entry inflated at most to its declared
//! size; triangles and component instances are counted per object before
//! anything is expanded, so a few kilobytes of nested components cannot
//! expand to millions of triangles.

use std::borrow::Cow;
use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{NamespaceResolver, QName, ResolveResult};
use quick_xml::reader::NsReader;

use super::{ImportError, ImportedMesh, MAX_TRIANGLES, Welder, unreadable, within_limit};
use crate::export::{ZipEntry, crc32, zip_entries};

const CORE_NS: &[u8] = b"http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const PRODUCTION_NS: &[u8] = b"http://schemas.microsoft.com/3dmanufacturing/production/2015/06";
/// Relationship type of the start part in `_rels/.rels`.
const MODEL_REL: &[u8] = b"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";
/// The start part when the archive names none.
const ROOT_MODEL: &str = "3D/3dmodel.model";
/// Declared size of all entries together.
const MAX_INFLATED: u64 = 512 * 1024 * 1024;
/// Component instances after expansion: real files have far fewer than
/// triangles, a hostile one could visit billions of empty instances.
const MAX_INSTANCES: u64 = 4 * MAX_TRIANGLES as u64;

/// Rows m0, m1, m2 (linear part) and m3 (translation).
type Affine = [[f64; 3]; 4];

const IDENTITY: Affine = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, 0.0]];

/// A component or build item: an object, in another model file with `path`.
struct Reference {
    path: Option<String>,
    object: u32,
    transform: Affine,
}

enum Object {
    Mesh { positions: Vec<[f64; 3]>, triangles: Vec<[u32; 3]> },
    Components(Vec<Reference>),
}

/// One model file of the archive. Object ids are scoped per file.
#[derive(Default)]
struct Model {
    /// mm per unit.
    unit_mm: f64,
    objects: HashMap<u32, Object>,
    build: Vec<Reference>,
}

/// Reads a 3MF archive: all build items, in mm.
///
/// # Errors
/// The archive or a model file is malformed, requires an extension other
/// than the production extension, inflates to more than 512 MiB, or
/// expands to no or more than [`MAX_TRIANGLES`] triangles.
pub fn read_3mf(bytes: &[u8]) -> Result<ImportedMesh, ImportError> {
    let entries = zip_entries(bytes).map_err(|e| unreadable(format!("not a readable 3MF: {e}")))?;
    let declared: u64 = entries.iter().map(|e| u64::from(e.size)).sum();
    if declared > MAX_INFLATED {
        return Err(unreadable(format!(
            "the archive inflates to {declared} bytes, more than 512 MiB"
        )));
    }
    let root = start_part(&entries)?;
    let mut models = HashMap::new();
    let mut pending = vec![root.clone()];
    while let Some(name) = pending.pop() {
        if models.contains_key(&name) {
            continue;
        }
        let bytes = entry(&entries, &name)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| unreadable(format!("{name}: not UTF-8: {e}")))?;
        let model = parse_model(text).map_err(|e| unreadable(format!("{name}: {e}")))?;
        let referenced = model.build.iter().chain(model.objects.values().flat_map(|o| match o {
            Object::Components(c) => c.as_slice(),
            Object::Mesh { .. } => &[],
        }));
        pending.extend(referenced.filter_map(|r| r.path.clone()));
        models.insert(name, model);
    }
    Expansion { models: &models, counts: HashMap::new() }.mesh(&root)
}

/// `/3D/x.model` → `3D/x.model`.
fn part_name(path: &str) -> String {
    path.trim_start_matches('/').to_owned()
}

/// The model the package relationships name, else `3D/3dmodel.model`.
fn start_part(entries: &[ZipEntry<'_>]) -> Result<String, ImportError> {
    if !entries.iter().any(|e| e.name == "_rels/.rels") {
        return Ok(ROOT_MODEL.to_owned());
    }
    let rels = entry(entries, "_rels/.rels")?;
    let text = std::str::from_utf8(&rels).map_err(|e| unreadable(format!("_rels/.rels: {e}")))?;
    let mut reader = NsReader::from_str(text);
    loop {
        let event = reader.read_event().map_err(|e| unreadable(format!("_rels/.rels: {e}")))?;
        match event {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                if attribute(&e, b"Type")?.is_some_and(|t| t.as_bytes() == MODEL_REL) {
                    let target = attribute(&e, b"Target")?.unwrap_or_default();
                    return Ok(part_name(&target));
                }
            }
            Event::Eof => return Ok(ROOT_MODEL.to_owned()),
            _ => {}
        }
    }
}

/// The inflated bytes of the entry `name`, CRC checked.
fn entry<'a>(entries: &[ZipEntry<'a>], name: &str) -> Result<Cow<'a, [u8]>, ImportError> {
    let e = entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| unreadable(format!("the archive has no {name}")))?;
    let data = match e.method {
        0 => Cow::Borrowed(e.data),
        8 => Cow::Owned(
            miniz_oxide::inflate::decompress_to_vec_with_limit(e.data, e.size as usize).map_err(
                |err| {
                    unreadable(format!(
                        "{name}: inflating failed or exceeds the declared {} bytes: {err}",
                        e.size
                    ))
                },
            )?,
        ),
        method => {
            return Err(unreadable(format!(
                "{name}: compression method {method} is not supported (only stored and deflate)"
            )));
        }
    };
    if data.len() != e.size as usize {
        return Err(unreadable(format!(
            "{name}: {} bytes instead of the declared {}",
            data.len(),
            e.size
        )));
    }
    if crc32(&data) != e.crc {
        return Err(unreadable(format!("{name}: CRC-32 mismatch, the entry is corrupted")));
    }
    Ok(data)
}

// ------------------------------------------------------------------ model XML

/// Where the parser is: the element whose content it reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    Model,
    Resources,
    Object,
    Mesh,
    Vertices,
    Triangles,
    Components,
    Build,
    /// Unknown elements and the content of elements read from attributes.
    Other,
}

#[derive(Default)]
struct Parser {
    stack: Vec<Context>,
    model: Model,
    /// The object being read: id and content.
    object: Option<(u32, Object)>,
    /// The root element has been closed.
    done: bool,
}

fn parse_model(text: &str) -> Result<Model, String> {
    let mut reader = NsReader::from_str(text);
    let mut parser = Parser::default();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("XML error near byte {}: {e}", reader.error_position()))?;
        match event {
            Event::Start(e) => {
                let context = parser.start(&e, reader.resolver())?;
                parser.stack.push(context);
            }
            Event::Empty(e) => {
                let context = parser.start(&e, reader.resolver())?;
                parser.end(context)?;
            }
            Event::End(_) => {
                let context = parser.stack.pop().ok_or("an end tag without a start tag")?;
                parser.end(context)?;
            }
            Event::Eof if parser.done => return Ok(parser.model),
            Event::Eof => return Err("the model ends before </model>".into()),
            _ => {}
        }
    }
}

impl Parser {
    /// Reads the start tag `e` and returns the context of its content.
    fn start(&mut self, e: &BytesStart<'_>, ns: &NamespaceResolver) -> Result<Context, String> {
        let (namespace, local) = ns.resolve_element(e.name());
        let core = matches!(namespace, ResolveResult::Bound(n) if n.as_ref() == CORE_NS);
        let name = if core { local.as_ref() } else { b"" };
        let parent = self.stack.last().copied();
        Ok(match (parent, name) {
            (None, _) if self.done => return Err("a second root element".into()),
            (None, b"model") => {
                let name = attribute_where(e, |name| name.as_ref() == b"unit")?;
                self.model.unit_mm = unit(name.as_deref())?;
                required_extensions(e, ns)?;
                Context::Model
            }
            (None, _) => return Err("the root element is not a 3MF <model>".into()),
            (Some(Context::Model), b"resources") => Context::Resources,
            (Some(Context::Model), b"build") => Context::Build,
            (Some(Context::Resources), b"object") => {
                let id = number(e, b"id", "object")?;
                self.object =
                    Some((id, Object::Mesh { positions: Vec::new(), triangles: Vec::new() }));
                Context::Object
            }
            (Some(Context::Object), b"mesh") => Context::Mesh,
            (Some(Context::Object), b"components") => {
                if let Some((_, object)) = &mut self.object {
                    *object = Object::Components(Vec::new());
                }
                Context::Components
            }
            (Some(Context::Mesh), b"vertices") => Context::Vertices,
            (Some(Context::Mesh), b"triangles") => Context::Triangles,
            (Some(Context::Vertices), b"vertex") => {
                let p = [coordinate(e, b"x")?, coordinate(e, b"y")?, coordinate(e, b"z")?];
                if let Some((_, Object::Mesh { positions, .. })) = &mut self.object {
                    positions.push(p);
                }
                Context::Other
            }
            (Some(Context::Triangles), b"triangle") => {
                let t = [b"v1", b"v2", b"v3"];
                let t = [
                    number(e, t[0], "triangle")?,
                    number(e, t[1], "triangle")?,
                    number(e, t[2], "triangle")?,
                ];
                if let Some((_, Object::Mesh { triangles, .. })) = &mut self.object {
                    triangles.push(t);
                }
                Context::Other
            }
            (Some(Context::Components), b"component") => {
                let component = reference(e, ns, "component")?;
                if let Some((_, Object::Components(list))) = &mut self.object {
                    list.push(component);
                }
                Context::Other
            }
            (Some(Context::Build), b"item") => {
                self.model.build.push(reference(e, ns, "item")?);
                Context::Other
            }
            _ => Context::Other,
        })
    }

    fn end(&mut self, context: Context) -> Result<(), String> {
        match context {
            Context::Model => self.done = true,
            Context::Object => {
                let (id, object) = self.object.take().ok_or("an object without content")?;
                if let Object::Mesh { positions, triangles } = &object {
                    let n = positions.len();
                    if let Some(t) = triangles.iter().find(|t| t.iter().any(|&i| i as usize >= n)) {
                        return Err(format!(
                            "object {id}: triangle {t:?} uses a vertex beyond the {n} vertices"
                        ));
                    }
                }
                if self.model.objects.insert(id, object).is_some() {
                    return Err(format!("object id {id} is used twice"));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// The unescaped value of the unprefixed attribute `key`.
fn attribute(e: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>, ImportError> {
    attribute_where(e, |name| name.as_ref() == key).map_err(unreadable)
}

fn attribute_where(
    e: &BytesStart<'_>,
    wanted: impl Fn(QName<'_>) -> bool,
) -> Result<Option<String>, String> {
    let element = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    for a in e.attributes() {
        let a = a.map_err(|err| format!("<{element}>: malformed attribute: {err}"))?;
        if wanted(a.key) {
            let value = a
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|err| format!("<{element}>: {err}"))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn required(e: &BytesStart<'_>, key: &[u8], what: &str) -> Result<String, String> {
    attribute_where(e, |name| name.as_ref() == key)?
        .ok_or_else(|| format!("{what} without the attribute {}", String::from_utf8_lossy(key)))
}

fn number(e: &BytesStart<'_>, key: &[u8], what: &str) -> Result<u32, String> {
    let text = required(e, key, what)?;
    text.trim().parse().map_err(|_| {
        format!("{what}: {}={text:?} is not a whole number >= 0", String::from_utf8_lossy(key))
    })
}

fn coordinate(e: &BytesStart<'_>, key: &[u8; 1]) -> Result<f64, String> {
    let text = required(e, key, "vertex")?;
    text.trim()
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite())
        .ok_or_else(|| format!("vertex: {}={text:?} is not a finite number", char::from(key[0])))
}

/// mm per unit of the `unit` attribute (3MF default: millimeter).
fn unit(name: Option<&str>) -> Result<f64, String> {
    Ok(match name.unwrap_or("millimeter") {
        "micron" => 0.001,
        "millimeter" => 1.0,
        "centimeter" => 10.0,
        "inch" => 25.4,
        "foot" => 304.8,
        "meter" => 1000.0,
        other => return Err(format!("unknown unit {other:?}")),
    })
}

/// Every prefix of `requiredextensions` must name the production extension.
fn required_extensions(e: &BytesStart<'_>, ns: &NamespaceResolver) -> Result<(), String> {
    let Some(list) = attribute_where(e, |name| name.as_ref() == b"requiredextensions")? else {
        return Ok(());
    };
    for prefix in list.split_ascii_whitespace() {
        let probe = format!("{prefix}:x");
        match ns.resolve_attribute(QName(probe.as_bytes())).0 {
            ResolveResult::Bound(n) if n.as_ref() == PRODUCTION_NS => {}
            ResolveResult::Bound(n) => {
                return Err(format!(
                    "requires the 3MF extension {}, which cannot be imported \
                     (only the production extension)",
                    String::from_utf8_lossy(n.as_ref())
                ));
            }
            _ => return Err(format!("requires the undeclared extension prefix {prefix:?}")),
        }
    }
    Ok(())
}

/// A `<component>` or `<item>`: objectid, optional transform and p:path.
fn reference(e: &BytesStart<'_>, ns: &NamespaceResolver, what: &str) -> Result<Reference, String> {
    let object = number(e, b"objectid", what)?;
    let path = attribute_where(e, |name| {
        let (namespace, local) = ns.resolve_attribute(name);
        local.as_ref() == b"path"
            && matches!(namespace, ResolveResult::Bound(n) if n.as_ref() == PRODUCTION_NS)
    })?;
    let transform = match attribute_where(e, |name| name.as_ref() == b"transform")? {
        Some(text) => matrix(&text)
            .ok_or_else(|| format!("{what}: transform {text:?} is not 12 finite numbers"))?,
        None => IDENTITY,
    };
    Ok(Reference { path: path.as_deref().map(part_name), object, transform })
}

fn matrix(text: &str) -> Option<Affine> {
    let numbers: Vec<f64> = text
        .split_ascii_whitespace()
        .map(|w| w.parse().ok().filter(|v: &f64| v.is_finite()))
        .collect::<Option<_>>()?;
    let m: &[f64; 12] = numbers.as_slice().try_into().ok()?;
    Some(std::array::from_fn(|r| [m[3 * r], m[3 * r + 1], m[3 * r + 2]]))
}

// ------------------------------------------------------------------ expansion

/// An object: its model file and id.
type Key<'m> = (&'m str, u32);

/// What an object expands to: triangles and component instances.
#[derive(Clone, Copy)]
enum Count {
    Visiting,
    Done { triangles: u64, instances: u64 },
}

struct Expansion<'m> {
    models: &'m HashMap<String, Model>,
    counts: HashMap<Key<'m>, Count>,
}

impl<'m> Expansion<'m> {
    /// The object `r` names, seen from the model file `from`.
    fn lookup(
        &self,
        from: &'m str,
        r: &'m Reference,
    ) -> Result<(Key<'m>, &'m Object), ImportError> {
        let file = r.path.as_deref().unwrap_or(from);
        let model = self
            .models
            .get(file)
            .ok_or_else(|| unreadable(format!("the archive has no {file}")))?;
        let object = model.objects.get(&r.object).ok_or_else(|| {
            unreadable(format!(
                "{file}: a component or item references object {}, which does not exist",
                r.object
            ))
        })?;
        Ok(((file, r.object), object))
    }

    fn children(object: &'m Object) -> &'m [Reference] {
        match object {
            Object::Components(c) => c,
            Object::Mesh { .. } => &[],
        }
    }

    /// Counts what `r` expands to, children first, without recursion (the
    /// nesting depth is the file's choice).
    fn count(&mut self, from: &'m str, r: &'m Reference) -> Result<(u64, u64), ImportError> {
        let mut stack = vec![(from, r, false)];
        while let Some((from, r, expanded)) = stack.pop() {
            let (key, object) = self.lookup(from, r)?;
            match (self.counts.get(&key), expanded) {
                (Some(Count::Done { .. }), _) => continue,
                (Some(Count::Visiting), false) => {
                    return Err(unreadable(format!(
                        "{}: object {} contains itself through its components",
                        key.0, key.1
                    )));
                }
                (_, false) => {
                    self.counts.insert(key, Count::Visiting);
                    stack.push((from, r, true));
                    stack.extend(Self::children(object).iter().map(|c| (key.0, c, false)));
                }
                (_, true) => {
                    let (mut triangles, mut instances) = match object {
                        Object::Mesh { triangles, .. } => (triangles.len() as u64, 1u64),
                        Object::Components(_) => (0, 1),
                    };
                    for c in Self::children(object) {
                        let (t, i) = self.counted(key.0, c)?;
                        triangles = triangles.saturating_add(t);
                        instances = instances.saturating_add(i);
                    }
                    self.counts.insert(key, Count::Done { triangles, instances });
                }
            }
        }
        self.counted(from, r)
    }

    fn counted(&self, from: &'m str, r: &'m Reference) -> Result<(u64, u64), ImportError> {
        let (key, _) = self.lookup(from, r)?;
        match self.counts.get(&key) {
            Some(Count::Done { triangles, instances }) => Ok((*triangles, *instances)),
            _ => Err(unreadable(format!("{}: object {} contains itself", key.0, key.1))),
        }
    }

    /// All build items of the start part `root`, welded, in mm.
    fn mesh(mut self, root: &'m str) -> Result<ImportedMesh, ImportError> {
        let model = &self.models[root];
        let (mut triangles, mut instances) = (0u64, 0u64);
        for item in &model.build {
            let (t, i) = self.count(root, item)?;
            triangles = triangles.saturating_add(t);
            instances = instances.saturating_add(i);
        }
        within_limit(triangles)?;
        if instances > MAX_INSTANCES {
            return Err(unreadable(format!(
                "the components expand to {instances} instances, more than {MAX_INSTANCES}"
            )));
        }
        let mut welder = Welder::default();
        let mut stack: Vec<(&str, &Reference, Affine)> =
            model.build.iter().rev().map(|item| (root, item, IDENTITY)).collect();
        while let Some((from, r, outer)) = stack.pop() {
            let (key, object) = self.lookup(from, r)?;
            // Component transform first, then the one of its parent.
            let transform = compose(&r.transform, &outer);
            match object {
                Object::Components(children) => {
                    stack.extend(children.iter().rev().map(|c| (key.0, c, transform)));
                }
                Object::Mesh { positions, triangles } => {
                    let mirrored = determinant(&transform) < 0.0;
                    for t in triangles {
                        let mut corners = t.map(|i| {
                            apply(&transform, positions[i as usize]).map(|c| c * model.unit_mm)
                        });
                        if mirrored {
                            corners.swap(1, 2);
                        }
                        welder.triangle(corners)?;
                    }
                }
            }
        }
        welder.finish()
    }
}

/// `p · m` in row-vector convention.
fn apply(m: &Affine, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| p[0] * m[0][k] + p[1] * m[1][k] + p[2] * m[2][k] + m[3][k])
}

/// First `a`, then `b`.
fn compose(a: &Affine, b: &Affine) -> Affine {
    let mut out = [[0.0; 3]; 4];
    for r in 0..3 {
        out[r] = std::array::from_fn(|k| (0..3).map(|j| a[r][j] * b[j][k]).sum());
    }
    out[3] = apply(b, a[3]);
    out
}

fn determinant(m: &Affine) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}
