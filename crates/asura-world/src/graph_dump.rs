//! The path-graph dump: `PathStaticData` written once, loaded without a `Gw.dat`.
//!
//! A map's path graph is derived data: `map_data::PathStaticData` is imported from one `Gw.dat`
//! blob, and after that the archive is only a *source*. This module is the portable form of that
//! data - a compact, versioned, little-endian file that `asura-world --path-graphs` can load at
//! startup so a server machine needs a few megabytes instead of a 3.6 GB client archive.
//!
//! It deliberately carries **exactly** what the reference's A* consumes (`GmPaths.c`): every plane,
//! its trapezoids with their neighbour and portal ids, its x/y split nodes and sink nodes, and its
//! portals with the paired portal on the other plane. A lossy form - the community's `.pmap` files,
//! for instance - keeps trapezoids and adjacency but drops the portals and the node tree, which the
//! finder needs; that is why this is our own format and not a pmap reader.
//!
//! ```text
//! magic     "ASURAPG1" (8 bytes)
//! version   u32 = 1
//! count     u32
//! per map:
//!   map_id        i32
//!   map_file_id   u32   (the Gw.dat blob id: the real key, maps share blobs)
//!   traps_count   u32
//!   plane count   u32, then each plane:
//!     plane_id u16
//!     vectors      u32 count, each x,y f32
//!     trapezoids   u32 count, each: trap_id u32, four neighbour ids (u32::MAX = none),
//!                  portal_left u16, portal_right u16, six corners f32
//!     xnodes       u32 count, each: pos x,y f32, dir x,y f32, left node, right node
//!     ynodes       u32 count, each: pos x,y f32, above node, bellow node
//!     sink_nodes   u32 count, each: trap u32
//!     portals      u32 count, each: plane ids u16 u16, flags u8, pair (0=none,1=then plane u16
//!                  + portal u32), traps_count u32, traps u32
//!     h000c u32
//!     portals_traps u32 count, each: u32 (u32::MAX = none)
//!     root_node    tag u8 (0 none, 1 X, 2 Y, 3 Sink) then index u32 when not none
//! ```
//!
//! A node reference (`NodeRef`) is written as that same tag + index.

use crate::map_data::{
    NodeRef, PathStaticData, Plane, Portal, SinkNode, Trapezoid, Vec2f, XNode, YNode,
};

/// The file's first eight bytes.
pub const MAGIC: &[u8; 8] = b"ASURAPG1";

/// The format version. A loader refuses anything else rather than guessing.
pub const VERSION: u32 = 1;

/// One dumped graph: the map it belongs to, the archive blob it came from, and the data.
pub struct DumpedGraph {
    /// The map id that was dumped, when known. `-1` for a whole-archive dump, where only the blob id
    /// is known - the world resolves the map id from its own catalogue by [`Self::map_file_id`].
    pub map_id: i32,
    /// The `Gw.dat` blob id: the real key of a graph, because several maps share one blob
    /// (146/148, 160/163, 161/165 and 162/166 in the reference's catalogue all do).
    pub map_file_id: u32,
    pub data: PathStaticData,
}

/// Write the graphs out.
pub fn encode(graphs: &[DumpedGraph]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(graphs.len() as u32).to_le_bytes());
    for graph in graphs {
        out.extend_from_slice(&graph.map_id.to_le_bytes());
        out.extend_from_slice(&graph.map_file_id.to_le_bytes());
        out.extend_from_slice(&graph.data.traps_count.to_le_bytes());
        out.extend_from_slice(&(graph.data.planes.len() as u32).to_le_bytes());
        for plane in &graph.data.planes {
            out.extend_from_slice(&plane.plane_id.to_le_bytes());
            out.extend_from_slice(&(plane.vectors.len() as u32).to_le_bytes());
            for vector in &plane.vectors {
                out.extend_from_slice(&vector.x.to_le_bytes());
                out.extend_from_slice(&vector.y.to_le_bytes());
            }
            out.extend_from_slice(&(plane.trapezoids.len() as u32).to_le_bytes());
            for trap in &plane.trapezoids {
                out.extend_from_slice(&trap.trap_id.to_le_bytes());
                out.extend_from_slice(&index_or_none(trap.top_left));
                out.extend_from_slice(&index_or_none(trap.top_right));
                out.extend_from_slice(&index_or_none(trap.bottom_left));
                out.extend_from_slice(&index_or_none(trap.bottom_right));
                out.extend_from_slice(&trap.portal_left.to_le_bytes());
                out.extend_from_slice(&trap.portal_right.to_le_bytes());
                for corner in [trap.yt, trap.yb, trap.xtl, trap.xtr, trap.xbl, trap.xbr] {
                    out.extend_from_slice(&corner.to_le_bytes());
                }
            }
            out.extend_from_slice(&(plane.xnodes.len() as u32).to_le_bytes());
            for node in &plane.xnodes {
                out.extend_from_slice(&node.pos.x.to_le_bytes());
                out.extend_from_slice(&node.pos.y.to_le_bytes());
                out.extend_from_slice(&node.dir.x.to_le_bytes());
                out.extend_from_slice(&node.dir.y.to_le_bytes());
                out.extend_from_slice(&node_ref_bits(node.left));
                out.extend_from_slice(&node_ref_bits(node.right));
            }
            out.extend_from_slice(&(plane.ynodes.len() as u32).to_le_bytes());
            for node in &plane.ynodes {
                out.extend_from_slice(&node.pos.x.to_le_bytes());
                out.extend_from_slice(&node.pos.y.to_le_bytes());
                out.extend_from_slice(&node_ref_bits(node.above));
                out.extend_from_slice(&node_ref_bits(node.bellow));
            }
            out.extend_from_slice(&(plane.sink_nodes.len() as u32).to_le_bytes());
            for node in &plane.sink_nodes {
                out.extend_from_slice(&(node.trap as u32).to_le_bytes());
            }
            out.extend_from_slice(&(plane.portals.len() as u32).to_le_bytes());
            for portal in &plane.portals {
                out.extend_from_slice(&portal.portal_plane_id.to_le_bytes());
                out.extend_from_slice(&portal.neighbor_plane_id.to_le_bytes());
                out.push(portal.flags);
                match portal.pair {
                    None => out.push(0),
                    Some((plane_id, at)) => {
                        out.push(1);
                        out.extend_from_slice(&plane_id.to_le_bytes());
                        out.extend_from_slice(&(at as u32).to_le_bytes());
                    }
                }
                out.extend_from_slice(&portal.traps_count.to_le_bytes());
                out.extend_from_slice(&(portal.traps as u32).to_le_bytes());
            }
            out.extend_from_slice(&plane.h000c.to_le_bytes());
            out.extend_from_slice(&(plane.portals_traps.len() as u32).to_le_bytes());
            for trap in &plane.portals_traps {
                out.extend_from_slice(&index_or_none(*trap));
            }
            match plane.root_node {
                None => out.push(0),
                Some(reference) => out.extend_from_slice(&node_ref_bits(Some(reference))),
            }
        }
    }
    out
}

/// Read the graphs back. Refuses anything that is not this format at this version.
pub fn decode(bytes: &[u8]) -> Result<Vec<DumpedGraph>, String> {
    let mut cursor = Cursor {
        bytes,
        at: 0,
        what: "the dump",
    };
    if bytes.len() < 8 || &bytes[..8] != MAGIC {
        return Err("not a path-graph dump (bad magic)".to_string());
    }
    cursor.at = 8;
    let version = cursor.u32()?;
    if version != VERSION {
        return Err(format!(
            "the dump is version {version}, this build reads {VERSION}"
        ));
    }
    let count = cursor.u32()? as usize;
    let mut graphs = Vec::with_capacity(count);
    for _ in 0..count {
        let map_id = cursor.i32()?;
        let map_file_id = cursor.u32()?;
        let traps_count = cursor.u32()?;
        let plane_count = cursor.u32()? as usize;
        let mut planes = Vec::with_capacity(plane_count);
        for _ in 0..plane_count {
            let plane_id = cursor.u16()?;
            let vector_count = cursor.u32()? as usize;
            let mut vectors = Vec::with_capacity(vector_count);
            for _ in 0..vector_count {
                vectors.push(Vec2f {
                    x: cursor.f32()?,
                    y: cursor.f32()?,
                });
            }
            let trap_count = cursor.u32()? as usize;
            let mut trapezoids = Vec::with_capacity(trap_count);
            for _ in 0..trap_count {
                let trap_id = cursor.u32()?;
                let top_left = index_or_absent(cursor.u32()?);
                let top_right = index_or_absent(cursor.u32()?);
                let bottom_left = index_or_absent(cursor.u32()?);
                let bottom_right = index_or_absent(cursor.u32()?);
                let portal_left = cursor.u16()?;
                let portal_right = cursor.u16()?;
                trapezoids.push(Trapezoid {
                    trap_id,
                    top_left,
                    top_right,
                    bottom_left,
                    bottom_right,
                    portal_left,
                    portal_right,
                    yt: cursor.f32()?,
                    yb: cursor.f32()?,
                    xtl: cursor.f32()?,
                    xtr: cursor.f32()?,
                    xbl: cursor.f32()?,
                    xbr: cursor.f32()?,
                });
            }
            let x_count = cursor.u32()? as usize;
            let mut xnodes = Vec::with_capacity(x_count);
            for _ in 0..x_count {
                xnodes.push(XNode {
                    pos: Vec2f {
                        x: cursor.f32()?,
                        y: cursor.f32()?,
                    },
                    dir: Vec2f {
                        x: cursor.f32()?,
                        y: cursor.f32()?,
                    },
                    left: cursor.node_ref()?,
                    right: cursor.node_ref()?,
                });
            }
            let y_count = cursor.u32()? as usize;
            let mut ynodes = Vec::with_capacity(y_count);
            for _ in 0..y_count {
                ynodes.push(YNode {
                    pos: Vec2f {
                        x: cursor.f32()?,
                        y: cursor.f32()?,
                    },
                    above: cursor.node_ref()?,
                    bellow: cursor.node_ref()?,
                });
            }
            let sink_count = cursor.u32()? as usize;
            let mut sink_nodes = Vec::with_capacity(sink_count);
            for _ in 0..sink_count {
                sink_nodes.push(SinkNode {
                    trap: cursor.u32()? as usize,
                });
            }
            let portal_count = cursor.u32()? as usize;
            let mut portals = Vec::with_capacity(portal_count);
            for _ in 0..portal_count {
                let portal_plane_id = cursor.u16()?;
                let neighbor_plane_id = cursor.u16()?;
                let flags = cursor.u8()?;
                let pair = match cursor.u8()? {
                    0 => None,
                    1 => Some((cursor.u16()?, cursor.u32()? as usize)),
                    other => return Err(format!("portal pair tag {other}")),
                };
                let traps_count = cursor.u32()?;
                let traps = cursor.u32()? as usize;
                portals.push(Portal {
                    portal_plane_id,
                    neighbor_plane_id,
                    flags,
                    pair,
                    traps_count,
                    traps,
                });
            }
            let h000c = cursor.u32()?;
            let portal_trap_count = cursor.u32()? as usize;
            let mut portals_traps = Vec::with_capacity(portal_trap_count);
            for _ in 0..portal_trap_count {
                portals_traps.push(index_or_absent(cursor.u32()?));
            }
            let root_node = cursor.node_ref()?;
            planes.push(Plane {
                plane_id,
                vectors,
                trapezoids,
                xnodes,
                ynodes,
                sink_nodes,
                portals,
                h000c,
                portals_traps,
                root_node,
            });
        }
        graphs.push(DumpedGraph {
            map_id,
            map_file_id,
            data: PathStaticData {
                planes,
                traps_count,
            },
        });
    }
    Ok(graphs)
}

/// `None` as `u32::MAX`, which no index reaches (a map with four billion trapezoids is not one).
fn index_or_none(index: Option<usize>) -> [u8; 4] {
    match index {
        Some(at) => (at as u32).to_le_bytes(),
        None => u32::MAX.to_le_bytes(),
    }
}

fn index_or_absent(bits: u32) -> Option<usize> {
    if bits == u32::MAX {
        None
    } else {
        Some(bits as usize)
    }
}

/// A node reference: one tag byte, then the index when not `None`.
fn node_ref_bits(reference: Option<NodeRef>) -> Vec<u8> {
    match reference {
        None => vec![0],
        Some(NodeRef::X(at)) => {
            let mut out = vec![1];
            out.extend_from_slice(&(at as u32).to_le_bytes());
            out
        }
        Some(NodeRef::Y(at)) => {
            let mut out = vec![2];
            out.extend_from_slice(&(at as u32).to_le_bytes());
            out
        }
        Some(NodeRef::Sink(at)) => {
            let mut out = vec![3];
            out.extend_from_slice(&(at as u32).to_le_bytes());
            out
        }
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    what: &'static str,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        if self.bytes.len() < self.at + count {
            return Err(format!(
                "{what} ends early at byte {}: wanted {count} more",
                self.at,
                what = self.what
            ));
        }
        let slice = &self.bytes[self.at..self.at + count];
        self.at += count;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.u32()?.to_le_bytes()))
    }

    fn node_ref(&mut self) -> Result<Option<NodeRef>, String> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(NodeRef::X(self.u32()? as usize))),
            2 => Ok(Some(NodeRef::Y(self.u32()? as usize))),
            3 => Ok(Some(NodeRef::Sink(self.u32()? as usize))),
            other => Err(format!("node reference tag {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_plane() -> PathStaticData {
        PathStaticData {
            traps_count: 7,
            planes: vec![
                Plane {
                    plane_id: 3,
                    vectors: vec![Vec2f { x: 1.5, y: 2.5 }, Vec2f { x: -3.0, y: 4.25 }],
                    trapezoids: vec![Trapezoid {
                        trap_id: 42,
                        top_left: None,
                        top_right: Some(1),
                        bottom_left: Some(2),
                        bottom_right: None,
                        portal_left: 9,
                        portal_right: 17,
                        yt: 10.0,
                        yb: 20.0,
                        xtl: 1.0,
                        xtr: 2.0,
                        xbl: 3.0,
                        xbr: 4.0,
                    }],
                    xnodes: vec![XNode {
                        pos: Vec2f { x: 5.0, y: 6.0 },
                        dir: Vec2f { x: 0.0, y: 1.0 },
                        left: Some(NodeRef::Y(2)),
                        right: Some(NodeRef::Sink(0)),
                    }],
                    ynodes: vec![YNode {
                        pos: Vec2f { x: 7.0, y: 8.0 },
                        above: None,
                        bellow: Some(NodeRef::X(1)),
                    }],
                    sink_nodes: vec![SinkNode { trap: 0 }],
                    portals: vec![Portal {
                        portal_plane_id: 3,
                        neighbor_plane_id: 4,
                        flags: 1,
                        pair: Some((4, 7)),
                        traps_count: 1,
                        traps: 2,
                    }],
                    h000c: 11,
                    portals_traps: vec![Some(0), None],
                    root_node: Some(NodeRef::X(0)),
                },
                Plane::default(),
            ],
        }
    }

    #[test]
    fn a_dump_round_trips_every_field() {
        let data = one_plane();
        let dumped = vec![DumpedGraph {
            map_id: 146,
            map_file_id: 0x1B97D,
            data: data.clone(),
        }];
        let bytes = encode(&dumped);
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(&bytes[8..12], &VERSION.to_le_bytes());
        let read = decode(&bytes).expect("decode");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].map_id, 146);
        assert_eq!(read[0].map_file_id, 0x1B97D);
        assert_eq!(read[0].data, data, "every field survives the round trip");
    }

    #[test]
    fn a_foreign_or_truncated_file_is_refused() {
        assert!(decode(b"not a dump").is_err());
        let bytes = encode(&[DumpedGraph {
            map_id: 146,
            map_file_id: 0x1B97D,
            data: one_plane(),
        }]);
        for cut in [8, 12, 20, bytes.len() / 2] {
            assert!(decode(&bytes[..cut]).is_err(), "a file cut at {cut}");
        }
        let mut wrong_version = bytes.clone();
        wrong_version[8..12].copy_from_slice(&99u32.to_le_bytes());
        assert!(
            decode(&wrong_version).is_err(),
            "a future version is refused"
        );
    }
}
