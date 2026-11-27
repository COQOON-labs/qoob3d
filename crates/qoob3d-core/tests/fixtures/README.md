# Test fixtures

## `bambu_two_objects.3mf`

Written by Bambu Studio 02.09.01.70 (`BambuStudio --export-3mf`) from two binary STLs that
`qoob3d export --format stl` wrote for parts of the reference project `models/`. It is a
third-party file: deflated ZIP entries, the production extension (`p:path` components in
`3D/Objects/object_*.model`), component and build item transforms, thumbnails and slicer
metadata.

| Object | Source | Shape | Size (mm) | Volume (mm³) | Faces | Edges |
| --- | --- | --- | --- | --- | --- | --- |
| `Block.stl` | `box(40, 30, 10)` | box | 40 × 30 × 10 | 12000 | 6 | 12 |
| `Angle.stl` | `extrude(polygon([[0, 0], [30, 0], [30, 4], [4, 4], [4, 20], [0, 20]]), 15)` | L-profile prism | 30 × 20 × 15 | 2760 | 8 | 18 |

Bambu Studio centres each mesh on its own origin and places it with the component and build
item transforms; both objects stand on z = 0.
