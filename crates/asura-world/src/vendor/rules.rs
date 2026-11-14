//! The trade rules, as pure functions: a window that is open, a request, what the character carries and
//! its gold in; a plan or a refusal out.
//!
//! No database, no socket, no file and nothing outside `std`, so every rule is a plain unit test. The
//! caller (`crate::vendor`) turns a [`Plan`] into inventory operations, one transaction and frames.
//!
//! Where each rule comes from:
//!
//! * a purchase costs `value x scale x 2` per item, and the client's quoted gold must equal it
//!   (preforged `gw_merchant_quote.h`, `buy_quote`; the live 4-gold purchase of a value-2 Flute,
//!   `packets-43292-168891734.jsonl` record 487667);
//! * a sale pays `value` per item, and the client's asked gold must equal it (preforged `sell_quote`;
//!   Rurik's eight retail sales, `docs/42-vendor-system-research.md` section 4);
//! * a collector takes exactly its cost in its own trophy, from stacks the request names, for one offer
//!   (preforged `gw_collector_transaction.h`, `authorize`);
//! * a gold move at a Xunlai window keeps the total and both caps (preforged `gw_inventory_gold.h`,
//!   `propose`).

use super::protocol::{TradeEntry, Transaction};

/// `0x804A` / `0x804D` kind of a purchase.
pub const KIND_BUY: u8 = 1;
/// Kind of a collector's exchange.
pub const KIND_COLLECTOR: u8 = 2;
/// Kind of a sale.
pub const KIND_SELL: u8 = 11;
/// The flags bit that makes an item stack (`ItemFlags` 0x80000, the bit preforged's stock reads).
pub const FLAG_STACKABLE: u32 = 0x0008_0000;

/// The values the rules run on, from the pack (`vendors.json` `rules`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// A merchant's price scale.
    pub buy_scale: f32,
    /// The most gold a character carries.
    pub carried_max: u32,
    /// The most gold storage holds.
    pub stored_max: u32,
    /// The largest stack a purchase may make.
    pub stack_max: u32,
}

/// One item a window offers: the id the window declared it under, and the pack's key for its row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    /// The id the window's `0x0161` and `0x0084` named.
    pub id: u32,
    /// The key of its row in `vendors.json`.
    pub key: String,
    /// The row's value.
    pub value: u32,
    /// The row's flags.
    pub flags: u32,
    /// For a collector's offer, the primary profession it is for (0 for any).
    pub profession: u8,
}

/// What a collector takes: the trophy, as the fields an owned item must match, and how many.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trophy {
    /// `item_type`.
    pub item_type: u8,
    /// `model_id`.
    pub model_id: u32,
    /// The coded name.
    pub name: Vec<u16>,
    /// How many one offer costs.
    pub cost: u32,
}

/// The window a player has open, as the server opened it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Window {
    /// A merchant: what it sells, and the models it will not buy.
    Merchant {
        /// The stock, in window order.
        stock: Vec<Offered>,
        /// Models a sale refuses.
        unsellable_models: Vec<u32>,
    },
    /// A collector: its trophy and its offers.
    Collector {
        /// The trophy and its cost.
        trophy: Trophy,
        /// The offers, in window order (the cost row is not one of them).
        offers: Vec<Offered>,
    },
}

/// One item the character owns, as the rules read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owned {
    /// The client-visible id.
    pub id: u32,
    /// It sits in a carrying bag (backpack, belt pouch, bag 1, bag 2).
    pub carried: bool,
    /// `item_type`.
    pub item_type: u8,
    /// `model_id`.
    pub model_id: u32,
    /// The coded name.
    pub name: Vec<u16>,
    /// The value a merchant pays for one.
    pub value: u32,
    /// How many are in the stack.
    pub quantity: u32,
}

/// What an accepted request does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Pay `price` and receive `quantity` of the stock row `key`, in one new stack.
    Buy {
        /// The stock row.
        key: String,
        /// How many.
        quantity: u32,
        /// The gold paid.
        price: u32,
        /// The new stack's limit: 1, or the stack maximum for a stackable row.
        stack_max: u32,
    },
    /// Take `quantity` of `item` and pay `price`.
    Sell {
        /// The item sold.
        item: u32,
        /// How many of it.
        quantity: u32,
        /// The whole stack goes.
        whole: bool,
        /// The gold received.
        price: u32,
    },
    /// Take the trophies and give one reward.
    Exchange {
        /// `(item, how many)` taken from each named stack, in request order.
        debits: Vec<(u32, u32)>,
        /// The reward row.
        key: String,
    },
}

/// Why a request was refused. Nothing changes and the client is not answered, as preforged answers a
/// refused trade (no live refusal is captured).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The request's kind is not one this window takes.
    WrongKind,
    /// The request's shape is not the kind's: lists of the wrong length, gold on the wrong side.
    Shape,
    /// The id it asks for is not one this window offered.
    NotOffered,
    /// A quantity of zero, past the stack, or more than one of an item that does not stack.
    Quantity,
    /// The client's gold figure is not the server's price.
    Price,
    /// Not enough gold to pay.
    Gold,
    /// The sale or the exchange would pass the carried-gold cap.
    Cap,
    /// The item named is not this character's, or not in a carrying bag.
    NotOwned,
    /// A merchant does not buy this item.
    NotSellable,
    /// A stack named for an exchange is not the collector's trophy, or is named twice.
    Trophy,
    /// The trophies named do not add up to the offer's cost.
    Cost,
    /// The offer is for another profession.
    Profession,
}

/// The gold one purchase costs: `value x scale x 2` per item, rounded half to even as the client's
/// rounder does (preforged `gw_merchant_quote.h`, `priced`). `None` past `u32`.
pub fn buy_price(value: u32, scale: f32, quantity: u32) -> Option<u32> {
    let unit = f64::from(value) * f64::from(scale) * 2.0;
    if !unit.is_finite() || unit < 0.0 {
        return None;
    }
    let floor = unit.floor();
    let fraction = unit - floor;
    let rounded = if fraction > 0.5 || (fraction == 0.5 && floor % 2.0 != 0.0) {
        floor + 1.0
    } else {
        floor
    };
    if rounded > f64::from(u32::MAX) {
        return None;
    }
    (rounded as u32).checked_mul(quantity)
}

/// The gold one sale pays: `value` per item.
pub fn sell_price(value: u32, quantity: u32) -> Option<u32> {
    value.checked_mul(quantity)
}

/// Decide a trade window's request.
pub fn authorize(
    window: &Window,
    request: &Transaction,
    owned: &[Owned],
    carried_gold: u32,
    primary: u8,
    limits: &Limits,
) -> Result<Plan, Refused> {
    match (window, request.kind) {
        (Window::Merchant { stock, .. }, KIND_BUY) => buy(stock, request, carried_gold, limits),
        (
            Window::Merchant {
                unsellable_models, ..
            },
            KIND_SELL,
        ) => sell(unsellable_models, request, owned, carried_gold, limits),
        (Window::Collector { trophy, offers }, KIND_COLLECTOR) => {
            exchange(trophy, offers, request, owned, primary)
        }
        _ => Err(Refused::WrongKind),
    }
}

fn buy(
    stock: &[Offered],
    request: &Transaction,
    carried_gold: u32,
    limits: &Limits,
) -> Result<Plan, Refused> {
    // A purchase gives gold and names one stock row (preforged: only the quantity form is used).
    if request.header != super::protocol::TRANSACT_ITEMS
        || !request.give.is_empty()
        || request.receive_gold != 0
        || request.receive.len() != 1
    {
        return Err(Refused::Shape);
    }
    let TradeEntry { item, quantity } = request.receive[0];
    let row = stock
        .iter()
        .find(|row| row.id == item)
        .ok_or(Refused::NotOffered)?;
    let stack_max = if row.flags & FLAG_STACKABLE != 0 {
        limits.stack_max
    } else {
        1
    };
    let quantity = u32::from(quantity);
    if quantity == 0 || quantity > stack_max {
        return Err(Refused::Quantity);
    }
    let price = buy_price(row.value, limits.buy_scale, quantity).ok_or(Refused::Price)?;
    if request.give_gold != price {
        return Err(Refused::Price);
    }
    if carried_gold < price {
        return Err(Refused::Gold);
    }
    Ok(Plan::Buy {
        key: row.key.clone(),
        quantity,
        price,
        stack_max,
    })
}

fn sell(
    unsellable_models: &[u32],
    request: &Transaction,
    owned: &[Owned],
    carried_gold: u32,
    limits: &Limits,
) -> Result<Plan, Refused> {
    if request.give_gold != 0 || !request.receive.is_empty() || request.give.len() != 1 {
        return Err(Refused::Shape);
    }
    let TradeEntry { item, quantity } = request.give[0];
    let held = owned
        .iter()
        .find(|owned| owned.id == item && owned.carried)
        .ok_or(Refused::NotOwned)?;
    if held.value == 0 || unsellable_models.contains(&held.model_id) {
        return Err(Refused::NotSellable);
    }
    // `0x804A` sells the whole stack and carries no quantity; `0x804D` names how many.
    let whole_stack = request.header == super::protocol::BUY_MATERIALS;
    let quantity = if whole_stack {
        if quantity != 0 {
            return Err(Refused::Shape);
        }
        held.quantity
    } else {
        u32::from(quantity)
    };
    if quantity == 0 || quantity > held.quantity {
        return Err(Refused::Quantity);
    }
    let price = sell_price(held.value, quantity).ok_or(Refused::Price)?;
    if request.receive_gold != price {
        return Err(Refused::Price);
    }
    if carried_gold
        .checked_add(price)
        .is_none_or(|sum| sum > limits.carried_max)
    {
        return Err(Refused::Cap);
    }
    Ok(Plan::Sell {
        item,
        quantity,
        whole: quantity == held.quantity,
        price,
    })
}

fn exchange(
    trophy: &Trophy,
    offers: &[Offered],
    request: &Transaction,
    owned: &[Owned],
    primary: u8,
) -> Result<Plan, Refused> {
    if request.give_gold != 0
        || request.receive_gold != 0
        || request.give.is_empty()
        || request.receive.len() != 1
    {
        return Err(Refused::Shape);
    }
    let whole_stacks = request.header == super::protocol::BUY_MATERIALS;
    let wanted = request.receive[0];
    // One of the offer: quantity 1 in the quantity form, 0 in the whole-stack form.
    if wanted.quantity != u8::from(!whole_stacks) {
        return Err(Refused::Quantity);
    }
    let offer = offers
        .iter()
        .find(|offer| offer.id == wanted.item)
        .ok_or(Refused::NotOffered)?;
    if offer.profession != 0 && offer.profession != primary {
        return Err(Refused::Profession);
    }
    let mut debits: Vec<(u32, u32)> = Vec::with_capacity(request.give.len());
    let mut total = 0u32;
    for entry in &request.give {
        if debits.iter().any(|(item, _)| *item == entry.item) {
            return Err(Refused::Trophy);
        }
        let held = owned
            .iter()
            .find(|owned| owned.id == entry.item && owned.carried)
            .ok_or(Refused::NotOwned)?;
        if held.item_type != trophy.item_type
            || held.model_id != trophy.model_id
            || held.name != trophy.name
        {
            return Err(Refused::Trophy);
        }
        let taken = if whole_stacks {
            if entry.quantity != 0 {
                return Err(Refused::Shape);
            }
            held.quantity
        } else {
            u32::from(entry.quantity)
        };
        if taken == 0 || taken > held.quantity {
            return Err(Refused::Quantity);
        }
        total = total.checked_add(taken).ok_or(Refused::Cost)?;
        if total > trophy.cost {
            return Err(Refused::Cost);
        }
        debits.push((entry.item, taken));
    }
    if total != trophy.cost {
        return Err(Refused::Cost);
    }
    Ok(Plan::Exchange {
        debits,
        key: offer.key.clone(),
    })
}

/// A Xunlai window's gold move: the client asks for two final balances. Accepted when the total is
/// kept and neither balance passes its cap; the answer is the new pair.
pub fn move_gold(
    current: (u32, u32),
    requested: (u32, u32),
    limits: &Limits,
) -> Result<(u32, u32), Refused> {
    let total = u64::from(current.0) + u64::from(current.1);
    if u64::from(requested.0) + u64::from(requested.1) != total {
        return Err(Refused::Price);
    }
    if requested.0 > limits.carried_max || requested.1 > limits.stored_max {
        return Err(Refused::Cap);
    }
    Ok(requested)
}

#[cfg(test)]
mod tests {
    use super::super::protocol::{BUY_MATERIALS, TRANSACT_ITEMS};
    use super::*;

    const LIMITS: Limits = Limits {
        buy_scale: 1.0,
        carried_max: 100_000,
        stored_max: 1_000_000,
        stack_max: 250,
    };

    fn offered(id: u32, key: &str, value: u32, flags: u32, profession: u8) -> Offered {
        Offered {
            id,
            key: key.to_string(),
            value,
            flags,
            profession,
        }
    }

    fn merchant() -> Window {
        Window::Merchant {
            stock: vec![
                offered(500, "focus_pre_searing", 50, 0x2000_0201, 0),
                offered(506, "flute", 2, 0x2000_0001, 0),
                offered(507, "stacked", 3, 0x2008_0001, 0),
            ],
            unsellable_models: vec![1101],
        }
    }

    fn request(
        header: u16,
        kind: u8,
        give_gold: u32,
        give: &[(u32, u8)],
        receive_gold: u32,
        receive: &[(u32, u8)],
    ) -> Transaction {
        let entries = |list: &[(u32, u8)]| {
            list.iter()
                .map(|&(item, quantity)| TradeEntry { item, quantity })
                .collect()
        };
        Transaction {
            header,
            kind,
            give_gold,
            give: entries(give),
            receive_gold,
            receive: entries(receive),
        }
    }

    fn owned(id: u32, value: u32, quantity: u32, model_id: u32) -> Owned {
        Owned {
            id,
            carried: true,
            item_type: 30,
            model_id,
            name: vec![10502],
            value,
            quantity,
        }
    }

    #[test]
    fn a_purchase_costs_twice_the_value_and_must_be_quoted_so() {
        // The live 4-gold Flute (value 2).
        let plan = authorize(
            &merchant(),
            &request(TRANSACT_ITEMS, KIND_BUY, 4, &[], 0, &[(506, 1)]),
            &[],
            10,
            1,
            &LIMITS,
        );
        assert_eq!(
            plan,
            Ok(Plan::Buy {
                key: "flute".into(),
                quantity: 1,
                price: 4,
                stack_max: 1
            })
        );
        let wrong = authorize(
            &merchant(),
            &request(TRANSACT_ITEMS, KIND_BUY, 3, &[], 0, &[(506, 1)]),
            &[],
            10,
            1,
            &LIMITS,
        );
        assert_eq!(wrong, Err(Refused::Price));
        let poor = authorize(
            &merchant(),
            &request(TRANSACT_ITEMS, KIND_BUY, 100, &[], 0, &[(500, 1)]),
            &[],
            99,
            1,
            &LIMITS,
        );
        assert_eq!(poor, Err(Refused::Gold));
        let unknown = authorize(
            &merchant(),
            &request(TRANSACT_ITEMS, KIND_BUY, 4, &[], 0, &[(999, 1)]),
            &[],
            10,
            1,
            &LIMITS,
        );
        assert_eq!(unknown, Err(Refused::NotOffered));
    }

    #[test]
    fn only_a_stackable_row_sells_more_than_one() {
        let two_flutes = authorize(
            &merchant(),
            &request(TRANSACT_ITEMS, KIND_BUY, 8, &[], 0, &[(506, 2)]),
            &[],
            10,
            1,
            &LIMITS,
        );
        assert_eq!(two_flutes, Err(Refused::Quantity));
        let ten = authorize(
            &merchant(),
            &request(TRANSACT_ITEMS, KIND_BUY, 60, &[], 0, &[(507, 10)]),
            &[],
            60,
            1,
            &LIMITS,
        );
        assert_eq!(
            ten,
            Ok(Plan::Buy {
                key: "stacked".into(),
                quantity: 10,
                price: 60,
                stack_max: 250
            })
        );
    }

    #[test]
    fn a_sale_pays_the_value_per_item_and_a_whole_stack_carries_no_quantity() {
        let carried = [owned(77, 3, 5, 429)];
        let part = authorize(
            &merchant(),
            &request(TRANSACT_ITEMS, KIND_SELL, 0, &[(77, 2)], 6, &[]),
            &carried,
            0,
            1,
            &LIMITS,
        );
        assert_eq!(
            part,
            Ok(Plan::Sell {
                item: 77,
                quantity: 2,
                whole: false,
                price: 6
            })
        );
        let whole = authorize(
            &merchant(),
            &request(BUY_MATERIALS, KIND_SELL, 0, &[(77, 0)], 15, &[]),
            &carried,
            0,
            1,
            &LIMITS,
        );
        assert_eq!(
            whole,
            Ok(Plan::Sell {
                item: 77,
                quantity: 5,
                whole: true,
                price: 15
            })
        );
        let asked_too_much = authorize(
            &merchant(),
            &request(BUY_MATERIALS, KIND_SELL, 0, &[(77, 0)], 16, &[]),
            &carried,
            0,
            1,
            &LIMITS,
        );
        assert_eq!(asked_too_much, Err(Refused::Price));
        let rich = authorize(
            &merchant(),
            &request(BUY_MATERIALS, KIND_SELL, 0, &[(77, 0)], 15, &[]),
            &carried,
            99_990,
            1,
            &LIMITS,
        );
        assert_eq!(rich, Err(Refused::Cap));
    }

    #[test]
    fn a_merchant_refuses_what_it_does_not_buy_and_what_is_not_carried() {
        let flute = [owned(5, 2, 1, 1101)];
        let free = [owned(6, 0, 1, 900)];
        let stored = [Owned {
            carried: false,
            ..owned(7, 9, 1, 900)
        }];
        for (items, id) in [(&flute[..], 5), (&free[..], 6)] {
            let answer = authorize(
                &merchant(),
                &request(BUY_MATERIALS, KIND_SELL, 0, &[(id, 0)], 2, &[]),
                items,
                0,
                1,
                &LIMITS,
            );
            assert_eq!(answer, Err(Refused::NotSellable));
        }
        let answer = authorize(
            &merchant(),
            &request(BUY_MATERIALS, KIND_SELL, 0, &[(7, 0)], 9, &[]),
            &stored,
            0,
            1,
            &LIMITS,
        );
        assert_eq!(answer, Err(Refused::NotOwned));
    }

    fn brownlow() -> Window {
        Window::Collector {
            trophy: Trophy {
                item_type: 30,
                model_id: 429,
                name: vec![10502],
                cost: 5,
            },
            offers: vec![
                offered(601, "belt_pouch_33", 5, 0, 0),
                offered(602, "krytan_robes_651", 9, 0, 6),
            ],
        }
    }

    #[test]
    fn a_collector_takes_exactly_its_cost_from_the_named_stacks() {
        let carried = [owned(10, 3, 3, 429), owned(11, 3, 4, 429)];
        let plan = authorize(
            &brownlow(),
            &request(
                TRANSACT_ITEMS,
                KIND_COLLECTOR,
                0,
                &[(10, 3), (11, 2)],
                0,
                &[(601, 1)],
            ),
            &carried,
            0,
            1,
            &LIMITS,
        );
        assert_eq!(
            plan,
            Ok(Plan::Exchange {
                debits: vec![(10, 3), (11, 2)],
                key: "belt_pouch_33".into()
            })
        );
        let short = authorize(
            &brownlow(),
            &request(
                TRANSACT_ITEMS,
                KIND_COLLECTOR,
                0,
                &[(10, 3)],
                0,
                &[(601, 1)],
            ),
            &carried,
            0,
            1,
            &LIMITS,
        );
        assert_eq!(short, Err(Refused::Cost));
        let twice = authorize(
            &brownlow(),
            &request(
                TRANSACT_ITEMS,
                KIND_COLLECTOR,
                0,
                &[(10, 3), (10, 2)],
                0,
                &[(601, 1)],
            ),
            &carried,
            0,
            1,
            &LIMITS,
        );
        assert_eq!(twice, Err(Refused::Trophy));
        let other_trophy = [owned(12, 3, 5, 433)];
        let wrong = authorize(
            &brownlow(),
            &request(
                TRANSACT_ITEMS,
                KIND_COLLECTOR,
                0,
                &[(12, 5)],
                0,
                &[(601, 1)],
            ),
            &other_trophy,
            0,
            1,
            &LIMITS,
        );
        assert_eq!(wrong, Err(Refused::Trophy));
    }

    #[test]
    fn a_profession_offer_is_only_for_that_profession() {
        let carried = [owned(10, 3, 5, 429)];
        let warrior = authorize(
            &brownlow(),
            &request(
                TRANSACT_ITEMS,
                KIND_COLLECTOR,
                0,
                &[(10, 5)],
                0,
                &[(602, 1)],
            ),
            &carried,
            0,
            1,
            &LIMITS,
        );
        assert_eq!(warrior, Err(Refused::Profession));
        let elementalist = authorize(
            &brownlow(),
            &request(
                TRANSACT_ITEMS,
                KIND_COLLECTOR,
                0,
                &[(10, 5)],
                0,
                &[(602, 1)],
            ),
            &carried,
            0,
            6,
            &LIMITS,
        );
        assert!(elementalist.is_ok());
    }

    #[test]
    fn a_window_takes_only_its_own_kind() {
        let answer = authorize(
            &brownlow(),
            &request(TRANSACT_ITEMS, KIND_BUY, 4, &[], 0, &[(601, 1)]),
            &[],
            10,
            1,
            &LIMITS,
        );
        assert_eq!(answer, Err(Refused::WrongKind));
    }

    #[test]
    fn a_gold_move_keeps_the_total_and_the_caps() {
        assert_eq!(move_gold((100, 50), (40, 110), &LIMITS), Ok((40, 110)));
        assert_eq!(
            move_gold((100, 50), (40, 111), &LIMITS),
            Err(Refused::Price)
        );
        assert_eq!(
            move_gold((100_000, 1_000_000), (0, 1_100_000), &LIMITS),
            Err(Refused::Cap)
        );
    }

    #[test]
    fn prices_round_half_to_even() {
        assert_eq!(buy_price(50, 1.0, 1), Some(100));
        assert_eq!(buy_price(5, 0.25, 1), Some(2)); // 2.5 -> 2
        assert_eq!(buy_price(7, 0.25, 1), Some(4)); // 3.5 -> 4
        assert_eq!(buy_price(u32::MAX, 1.0, 1), None);
        assert_eq!(sell_price(3, 5), Some(15));
    }
}
