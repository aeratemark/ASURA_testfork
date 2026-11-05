//! A character's explored blocks: the read and the write, one continent at a time.
//!
//! The grid itself - what a block is, how the bits are packed, and how they compress onto the wire -
//! belongs to `crates/asura-world/src/cartography.rs`. This module only moves the bytes, so that the
//! world crate owns the format and this one owns the row. Migration 0027 has the reasoning for the
//! shape: per character because that is where the game keeps exploration, per continent because that
//! is what one bitmap covers, packed bits rather than the wire stream because the stream's polarity
//! is still a reading and would reinterpret every stored row if it changed.
//!
//! Both calls are one statement. Exploration is read once when a connection is built and written once
//! when it ends, beside the position (`crates/asura-world/src/connection.rs`), so neither is on a hot
//! path and neither needs a transaction: a row is self-contained and there is nothing to keep in step
//! with it.

use crate::{PgPool, Uuid};

/// One continent's exploration for one character, as the row holds it.
///
/// `blocks` is one bit per block, row-major and LSB first within each byte, against the `dims` beside
/// it - and the dims are part of the value rather than assumed, because they are what gives the bits a
/// shape and only continent 1's are measured. A caller that cannot match them to the grid it is about
/// to send should treat the row as absent rather than reshape it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explored {
    /// The grid's width, in cells.
    pub dims_x: i16,
    /// The grid's height, in cells.
    pub dims_y: i16,
    /// One bit per cell, row-major, LSB first within each byte.
    pub blocks: Vec<u8>,
    /// How many bits are set: the number a Cartographer percentage comes from.
    pub explored: i32,
}

impl Explored {
    /// Whether the row's own numbers agree: the blob is the size its dims ask for, and the count is
    /// not larger than the blocks there are.
    ///
    /// A row that fails this is corrupt rather than stale, and a caller should say so rather than
    /// draw it. Nothing in this crate can produce one; a hand-written `UPDATE` can.
    pub fn is_self_consistent(&self) -> bool {
        if self.dims_x <= 0 || self.dims_y <= 0 || self.explored < 0 {
            return false;
        }
        let blocks = self.dims_x as usize * self.dims_y as usize;
        self.blocks.len() == blocks.div_ceil(8) && self.explored as usize <= blocks
    }
}

/// The columns [`Explored`] reads. One list, so the two statements cannot disagree about the order.
type Row = (i16, i16, Vec<u8>, i32);

/// What this character has explored of `continent`, if anything has been stored for it.
///
/// `None` is a character that has never explored this continent, which is also what a character
/// created before migration 0027 looks like: a fresh grid, and the same all-fogged map the server
/// sent everybody before it could send anything else.
pub async fn explored(
    pool: &PgPool,
    character: Uuid,
    continent: i16,
) -> sqlx::Result<Option<Explored>> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT dims_x, dims_y, blocks, explored
           FROM t_character_cartography
          WHERE character_id = $1 AND continent = $2",
    )
    .bind(character)
    .bind(continent)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(dims_x, dims_y, blocks, explored)| Explored {
        dims_x,
        dims_y,
        blocks,
        explored,
    }))
}

/// Store what this character has explored of `continent`, replacing whatever was there.
///
/// Replacing rather than merging, and the caller is what makes that safe: the grid it hands over was
/// read from this same row when the connection began and has only had blocks added to it since, so
/// the write is the row plus this session's progress. Two connections for one character are the case
/// this does not cover - the later write wins - and the only thing that opens two is a travel, where
/// the arriving connection has just read what the leaving one is about to write.
pub async fn record_explored(
    pool: &PgPool,
    character: Uuid,
    continent: i16,
    grid: &Explored,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO t_character_cartography
                (character_id, continent, dims_x, dims_y, blocks, explored, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, now())
         ON CONFLICT (character_id, continent) DO UPDATE
            SET dims_x = $3, dims_y = $4, blocks = $5, explored = $6, updated_at = now()",
    )
    .bind(character)
    .bind(continent)
    .bind(grid.dims_x)
    .bind(grid.dims_y)
    .bind(grid.blocks.clone())
    .bind(grid.explored)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(dims: (i16, i16), explored: i32) -> Explored {
        let blocks = dims.0 as usize * dims.1 as usize;
        Explored {
            dims_x: dims.0,
            dims_y: dims.1,
            blocks: vec![0; blocks.div_ceil(8)],
            explored,
        }
    }

    #[test]
    fn a_row_this_crate_could_write_is_self_consistent() {
        assert!(row((64, 128), 0).is_self_consistent());
        assert!(row((64, 128), 64 * 128).is_self_consistent());
        assert!(row((256, 192), 947).is_self_consistent());
    }

    #[test]
    fn a_row_whose_numbers_disagree_is_not() {
        let mut short = row((64, 128), 0);
        short.blocks.pop();
        assert!(!short.is_self_consistent(), "the blob is shorter than its dims");

        let mut overcounted = row((64, 128), 0);
        overcounted.explored = 64 * 128 + 1;
        assert!(!overcounted.is_self_consistent(), "more set bits than blocks");

        assert!(!row((0, 128), 0).is_self_consistent(), "no width");
        assert!(!row((64, 128), -1).is_self_consistent(), "a negative count");
    }
}
