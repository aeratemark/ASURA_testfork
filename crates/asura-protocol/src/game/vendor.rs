//! The vendor family: an NPC's trade window (merchant, collector), its transactions, the gold the
//! character carries and the gold in storage, and the Xunlai storage window.
//!
//! Every server message here is measured on a live server unless its doc says otherwise. Two
//! recordings answer most of it:
//!
//! * `recordings/client/feature/movement/movement-live-20261109-200703.jsonl` (this repository, a live
//!   server, both directions): a dialog, a `SEND_DIALOG` choice and the merchant window it opens
//!   (seq 5171-5288);
//! * preforged's `data/captures/pre-searing-side-quests-2026-10-12/checkpoint-1815/packets-43292-168891734.jsonl`
//!   (a live pre-Searing session on this build's numbering, server-to-client only, the client's decoded
//!   message structs): seventeen trade windows, one purchase and one collector exchange, and the gold
//!   messages of every load. Line numbers below count packet records in that file from 0
//!   (`docs/53-npc-services.md` says how to reproduce them).
//!
//! What a client sends is preforged's reading of the 2022 client (`gw_collector_transaction.h`,
//! `gw_inventory_gold.h`), matched to this build's table by layout: preforged's client message 72 is
//! `0x804A` here, 75 is `0x804D` and 122 is `0x807C` (preforged's number plus two, `docs/52-skills.md`).

use super::Builder;
use crate::packets::game as msg;

// ---- the window kinds (`0x00C3` field 1, `0x00CC` field 1) --------------------------------------------

/// `0x00C3` and `0x00CC` kind: a merchant's sell tab, and a sale's completion. Immutable: the client's own
/// number (`docs/42-vendor-system-research.md` section 2; live `0x00C3 {11, 0}` at seq 5286).
pub const KIND_SELL: u8 = 11;
/// `0x00CC` kind: a purchase's completion (live capture line 487670: `0x00CC {1}` after a buy).
pub const KIND_BUY: u8 = 1;
/// `0x00CA` kind of a merchant's buy pane (live `0x00CA {1, 1.0}` at seq 5285 and capture line 487616).
pub const BUY_PANE_MERCHANT: u8 = 1;
/// `0x00C5` and `0x00CC` kind: a collector's offer (live capture line 537235 `0x00C5 {2, heading}` and
/// line 537266 `0x00CC {2}` after the exchange).
pub const KIND_COLLECTOR: u8 = 2;

/// `GAME_SMSG_AGENT_PROPERTY_UPDATE_INT` kind the live server sends on the NPC after every answer to a
/// talk, value 0: after a dialog (seq 5185, 5270), after a merchant window (seq 5288), after a collector
/// window (capture line 537236). Its meaning is not established; it is copied.
pub const PROPERTY_TALKED_TO: u32 = 12;

/// The unnamed `0x00CA`: a byte and a dword. Rurik ran it as the message that draws a merchant's buy pane
/// (`docs/42-vendor-system-research.md` section 1); the live server sends `{1, 1.0f}` after a merchant's
/// stock list and `{3, 1.0f}` for another trader (capture line 460410).
pub const WINDOW_BUY_PANE: u16 = 0x00CA;
/// The unnamed `0x00C5`: a byte and a coded string. A collector's window: kind 2 and the line the
/// collector says over its offers (capture line 537235).
pub const WINDOW_COLLECTOR: u16 = 0x00C5;

// ---- the window --------------------------------------------------------------------------------------

/// `GAME_SMSG_MERCHANT_WINDOW_UPDATE_OWNER` (`0x00C4`): the agent the window belongs to. First in every
/// trade window (seq 5281; capture lines 9835, 10070, 487607).
pub fn window_owner(agent_id: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_MERCHANT_WINDOW_UPDATE_OWNER)
        .u32(agent_id)
        .frame()
}

/// `GAME_SMSG_WINDOW_ADD_ITEMS` (`0x0084`): the ids of the items the window offers, each already declared
/// by `0x0161`, in the order they are listed (seq 5284: two ids after two `0x0161`).
pub fn window_add_items(item_ids: &[u32]) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_WINDOW_ADD_ITEMS)
        .dwords(item_ids)
        .frame()
}

/// `0x00CA`: the buy pane, its kind and the price scale the client multiplies an item's value by before
/// its own doubling. `{1, 1.0}` in every live merchant window.
pub fn window_buy_pane(kind: u8, scale: f32) -> crate::client::Frame {
    Builder::new(WINDOW_BUY_PANE).u8(kind).f32(scale).frame()
}

/// `GAME_SMSG_MERCHANT_WINDOW_OPEN` (`0x00C3`): the window's second tab. `{11, 0}` is the merchant's sell
/// tab, sent last (seq 5286; capture line 10080).
pub fn merchant_window_open(kind: u8, data: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_MERCHANT_WINDOW_OPEN)
        .u8(kind)
        .u32(data)
        .frame()
}

/// `0x00C5`: a collector's window, its kind (2) and the line it says over its offers.
pub fn window_collector(kind: u8, heading: &[u16]) -> crate::client::Frame {
    Builder::new(WINDOW_COLLECTOR)
        .u8(kind)
        .units(heading, 122)
        .frame()
}

/// The property the live server sends on the NPC after an answer to a talk (see [`PROPERTY_TALKED_TO`]).
pub fn talked_to(npc_agent: u32) -> crate::client::Frame {
    super::agent_property_update_int(PROPERTY_TALKED_TO, npc_agent, 0)
}

// ---- gold -------------------------------------------------------------------------------------------

/// `GAME_SMSG_UPDATE_GOLD_CHARACTER` (`0x0140`): add this much to the gold the character carries. A
/// delta, not a balance: the load sends the carried gold with it after the weapon sets (capture line
/// 472025: `{stream, 6}` then `0x0141`), a pickup sends what was picked up (line 468985).
pub fn gold_character_add(stream: u16, amount: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_UPDATE_GOLD_CHARACTER)
        .u16(stream)
        .u32(amount)
        .frame()
}

/// `GAME_SMSG_UPDATE_GOLD_STORAGE` (`0x0141`): add this much to the gold in storage. The load sends the
/// stored gold with it, right after the carried gold (capture line 472026: `{stream, 1000000}`).
pub fn gold_storage_add(stream: u16, amount: u32) -> crate::client::Frame {
    super::update_gold_storage(stream, amount)
}

/// `GAME_SMSG_GOLD_CHARACTER_REMOVE` (`0x014F`): take this much off the carried gold. A purchase pays
/// with it, before the item exists (capture line 487667: `{stream, 4}` for a 4-gold buy).
pub fn gold_character_remove(stream: u16, amount: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_GOLD_CHARACTER_REMOVE)
        .u16(stream)
        .u32(amount)
        .frame()
}

/// `GAME_SMSG_GOLD_STORAGE_REMOVE` (`0x0150`): take this much off the stored gold. Not captured; the
/// layout is the table's and the meaning preforged's (its message 335, `gw_inventory_gold.h`).
pub fn gold_storage_remove(stream: u16, amount: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_GOLD_STORAGE_REMOVE)
        .u16(stream)
        .u32(amount)
        .frame()
}

/// The deltas that take a client from one pair of balances to another: debits before credits, so the
/// client never shows gold that does not exist yet (preforged `gw_inventory_gold.h`, `updates`). Nothing
/// for a balance that does not change.
pub fn gold_deltas(
    stream: u16,
    before: (u32, u32),
    after: (u32, u32),
) -> Vec<crate::client::Frame> {
    let (carried_before, stored_before) = before;
    let (carried_after, stored_after) = after;
    let mut frames = Vec::new();
    if carried_after < carried_before {
        frames.push(gold_character_remove(
            stream,
            carried_before - carried_after,
        ));
    }
    if stored_after < stored_before {
        frames.push(gold_storage_remove(stream, stored_before - stored_after));
    }
    if carried_after > carried_before {
        frames.push(gold_character_add(stream, carried_after - carried_before));
    }
    if stored_after > stored_before {
        frames.push(gold_storage_add(stream, stored_after - stored_before));
    }
    frames
}

// ---- Xunlai storage -----------------------------------------------------------------------------------

/// `GAME_SMSG_WINDOW_OPEN` (`0x0083`) kind of the Xunlai storage window.
pub const WINDOW_STORAGE: u8 = 0;
/// The storage window's permission word: bit 1 deposit, bit 2 withdraw. Preforged's reading of the
/// client's handler (`gw_storage.h`, `open_window`), run against a client that opened, deposited into and
/// withdrew from storage (`docs/storage-pane-provisioning.txt` there). Not captured on a live server.
pub const STORAGE_DEPOSIT_AND_WITHDRAW: u32 = 3;

/// The permission word a dialog choice's window carries on the live server: `0xCCCCCCCC` at
/// `recordings/client/feature/movement/movement-live-20261109-200703.jsonl` seq 5247. The meaning of the
/// value is not established; it is copied.
pub const WINDOW_DIALOG_PERMISSION: u32 = 0xCCCC_CCCC;

/// `GAME_SMSG_WINDOW_OPEN` (`0x0083`): open a window owned by `agent_id`, of `kind`, with a permission
/// word. The Xunlai storage window is kind 0; the live dialog choice at seq 5247 opened kind 5.
pub fn window_open(agent_id: u32, kind: u8, permissions: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_WINDOW_OPEN)
        .u32(agent_id)
        .u8(kind)
        .u32(permissions)
        .frame()
}

/// `GAME_SMSG_WINDOW_OPEN` (`0x0083`): open the Xunlai storage window, owned by `agent_id`.
pub fn storage_window_open(agent_id: u32) -> crate::client::Frame {
    window_open(agent_id, WINDOW_STORAGE, STORAGE_DEPOSIT_AND_WITHDRAW)
}

// ---- the requests ------------------------------------------------------------------------------------

/// One item a transaction names, and how many of it. A full-stack request (`0x804A`) carries no
/// quantities; its entries read 0 here, and 0 means "all of it".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeEntry {
    /// The item id: an owned item on the give side, a window's offered id on the receive side.
    pub item: u32,
    /// How many; 0 in a `0x804A` request, where the whole stack is meant.
    pub quantity: u8,
}

/// A trade window's request: `0x804A` (whole stacks) or `0x804D` (with quantities).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// The header it came on.
    pub header: u16,
    /// The window kind: 1 buy, 11 sell, 2 collector.
    pub kind: u8,
    /// The gold the client gives (a purchase's price).
    pub give_gold: u32,
    /// What the client gives (a sale's item, a collector's trophies).
    pub give: Vec<TradeEntry>,
    /// The gold the client expects back (a sale's price).
    pub receive_gold: u32,
    /// What the client wants (a purchase's stock id, a collector's offer id).
    pub receive: Vec<TradeEntry>,
}

/// `GAME_CMSG_TRANSACT_ITEMS` (`0x804D`): a trade request with a quantity per id.
pub const TRANSACT_ITEMS: u16 = msg::GAME_CMSG_TRANSACT_ITEMS;
/// `GAME_CMSG_BUY_MATERIALS` (`0x804A`): a trade request for whole stacks. The table's name is the
/// reference's; a merchant sale and a collector exchange use it too (`docs/53-npc-services.md`).
pub const BUY_MATERIALS: u16 = msg::GAME_CMSG_BUY_MATERIALS;

/// The most ids one side of a request can carry (`Array32` length 16 in both rows).
pub const TRADE_ENTRIES: usize = 16;

struct Reader<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let value = *self.body.get(self.at)?;
        self.at += 1;
        Some(value)
    }

    fn u16(&mut self) -> Option<u16> {
        let bytes: [u8; 2] = self.body.get(self.at..self.at + 2)?.try_into().ok()?;
        self.at += 2;
        Some(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Option<u32> {
        let bytes: [u8; 4] = self.body.get(self.at..self.at + 4)?.try_into().ok()?;
        self.at += 4;
        Some(u32::from_le_bytes(bytes))
    }

    /// One side of a request: a counted list of ids and, for `0x804D`, a counted list of byte
    /// quantities of the same length.
    fn side(&mut self, with_quantities: bool) -> Option<Vec<TradeEntry>> {
        let count = usize::from(self.u16()?);
        if count > TRADE_ENTRIES {
            return None;
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(TradeEntry {
                item: self.u32()?,
                quantity: 0,
            });
        }
        if with_quantities {
            if usize::from(self.u16()?) != count {
                return None;
            }
            for entry in &mut entries {
                entry.quantity = self.u8()?;
            }
        }
        Some(entries)
    }

    fn done(&self) -> bool {
        self.at == self.body.len()
    }
}

/// Read a `0x804A` (`GAME_CMSG_BUY_MATERIALS`) or `0x804D` (`GAME_CMSG_TRANSACT_ITEMS`) body. `None` for
/// any other header, a list longer than the row allows, a quantity list whose length differs from its id
/// list, or bytes left over.
pub fn read_transaction(header: u16, body: &[u8]) -> Option<Transaction> {
    let with_quantities = match header {
        msg::GAME_CMSG_TRANSACT_ITEMS => true,
        msg::GAME_CMSG_BUY_MATERIALS => false,
        _ => return None,
    };
    let mut reader = Reader { body, at: 0 };
    let kind = reader.u8()?;
    let give_gold = reader.u32()?;
    let give = reader.side(with_quantities)?;
    let receive_gold = reader.u32()?;
    let receive = reader.side(with_quantities)?;
    reader.done().then_some(Transaction {
        header,
        kind,
        give_gold,
        give,
        receive_gold,
        receive,
    })
}

/// Read a `0x807C` (`GAME_CMSG_ITEM_CHANGE_GOLD`) body: the carried and the stored balance the client asks
/// to end with. Both are final balances, not amounts (preforged `gw_inventory_gold.h`, `parse`).
pub fn read_change_gold(body: &[u8]) -> Option<(u32, u32)> {
    let mut reader = Reader { body, at: 0 };
    let carried = reader.u32()?;
    let stored = reader.u32()?;
    reader.done().then_some((carried, stored))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(frame: &crate::client::Frame) -> Vec<u8> {
        frame.encode()
    }

    #[test]
    fn a_merchant_window_is_the_live_order_and_widths() {
        assert_eq!(bytes(&window_owner(0x36)), vec![0xC4, 0x00, 0x36, 0, 0, 0]);
        assert_eq!(
            bytes(&window_add_items(&[0x46D, 0x45E])),
            vec![0x84, 0x00, 2, 0, 0x6D, 0x04, 0, 0, 0x5E, 0x04, 0, 0]
        );
        assert_eq!(
            bytes(&window_buy_pane(BUY_PANE_MERCHANT, 1.0)),
            vec![0xCA, 0x00, 1, 0x00, 0x00, 0x80, 0x3F]
        );
        assert_eq!(
            bytes(&merchant_window_open(KIND_SELL, 0)),
            vec![0xC3, 0x00, 11, 0, 0, 0, 0]
        );
        assert_eq!(
            bytes(&talked_to(0x36)),
            vec![0x9F, 0x00, 12, 0, 0, 0, 0x36, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn a_collector_window_carries_its_kind_and_heading() {
        // Capture line 537235: Brownlow's line over Skale Fins, five each.
        let heading = [
            0x2B33, 0xD6E2, 0xD8C7, 0x0A6A, 0x010A, 0x2906, 0x0001, 0x0101, 0x0105,
        ];
        let wire = bytes(&window_collector(KIND_COLLECTOR, &heading));
        assert_eq!(&wire[..5], &[0xC5, 0x00, 2, 9, 0]);
        assert_eq!(&wire[5..7], &[0x33, 0x2B]);
        assert_eq!(wire.len(), 5 + 18);
    }

    #[test]
    fn gold_moves_debit_first_and_skip_what_does_not_change() {
        let frames = gold_deltas(1, (100, 50), (40, 110));
        let headers: Vec<u16> = frames.iter().map(|frame| frame.header()).collect();
        assert_eq!(
            headers,
            vec![
                msg::GAME_SMSG_GOLD_CHARACTER_REMOVE,
                msg::GAME_SMSG_UPDATE_GOLD_STORAGE
            ]
        );
        assert_eq!(bytes(&frames[0]), vec![0x4F, 0x01, 1, 0, 60, 0, 0, 0]);
        assert!(gold_deltas(1, (7, 7), (7, 7)).is_empty());
    }

    #[test]
    fn the_storage_window_names_its_owner_and_both_permissions() {
        assert_eq!(
            bytes(&storage_window_open(0x2A)),
            vec![0x83, 0x00, 0x2A, 0, 0, 0, 0, 3, 0, 0, 0]
        );
    }

    #[test]
    fn a_dialog_choice_window_copies_the_live_kind_and_permission() {
        // seq 5247: agent 0x36, kind 5, permission 0xCCCCCCCC. The struct layer pads the kind; the wire
        // is one byte, as `0x0083`'s 11 bytes say.
        assert_eq!(
            bytes(&window_open(0x36, 5, WINDOW_DIALOG_PERMISSION)),
            vec![0x83, 0x00, 0x36, 0, 0, 0, 5, 0xCC, 0xCC, 0xCC, 0xCC]
        );
    }

    #[test]
    fn a_purchase_request_reads_its_price_and_its_stock_id() {
        // Rurik's retail purchase: [1, 40, [], b'', 0, [2474], b'\x01'] (docs/42 section 3).
        let mut body = vec![1u8];
        body.extend_from_slice(&40u32.to_le_bytes());
        body.extend_from_slice(&[0, 0, 0, 0]); // no ids, no quantities
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&[1, 0]);
        body.extend_from_slice(&2474u32.to_le_bytes());
        body.extend_from_slice(&[1, 0, 1]);
        let request = read_transaction(msg::GAME_CMSG_TRANSACT_ITEMS, &body).expect("a request");
        assert_eq!(request.kind, KIND_BUY);
        assert_eq!(request.give_gold, 40);
        assert!(request.give.is_empty());
        assert_eq!(
            request.receive,
            vec![TradeEntry {
                item: 2474,
                quantity: 1
            }]
        );
        // One byte too many is not a request.
        let mut longer = body.clone();
        longer.push(0);
        assert!(read_transaction(msg::GAME_CMSG_TRANSACT_ITEMS, &longer).is_none());
    }

    #[test]
    fn a_whole_stack_sale_has_no_quantities() {
        // Rurik's retail sale: [11, 0, [item id], price, []].
        let mut body = vec![KIND_SELL];
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&[1, 0]);
        body.extend_from_slice(&77u32.to_le_bytes());
        body.extend_from_slice(&20u32.to_le_bytes());
        body.extend_from_slice(&[0, 0]);
        let request = read_transaction(msg::GAME_CMSG_BUY_MATERIALS, &body).expect("a request");
        assert_eq!(
            request.give,
            vec![TradeEntry {
                item: 77,
                quantity: 0
            }]
        );
        assert_eq!(request.receive_gold, 20);
        assert!(request.receive.is_empty());
    }

    #[test]
    fn a_gold_change_is_two_final_balances() {
        let mut body = 25u32.to_le_bytes().to_vec();
        body.extend_from_slice(&75u32.to_le_bytes());
        assert_eq!(read_change_gold(&body), Some((25, 75)));
        assert_eq!(read_change_gold(&body[..7]), None);
    }
}
