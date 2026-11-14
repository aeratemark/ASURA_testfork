//! Guild Wars item modifier words: what a word says, without changing what it is.
//!
//! A modifier is one `u32` in `CREATE_NAMED_ITEM`'s trailing array. The raw word is the only thing
//! every source agrees on, so [`Modifier`] always carries it and [`Modifier::raw`] returns it
//! unchanged. Known words gain an [`ModifierEffect`]; an unknown word, a skipped word and a word
//! this module has no evidence for stay [`ModifierEffect::Unknown`] and survive persistence and
//! serialization exactly as they arrived.
//!
//! ## The two views of the same bits
//!
//! The reference's own client helpers and Headquarter read the word as a 16-bit id plus two
//! bytes (`item.c:6-19`):
//!
//! ```text
//! id   = raw >> 16
//! arg1 = (raw >> 8) & 0xFF
//! arg2 = raw & 0xFF
//! ```
//!
//! Rurik's client read established what the id contains: bits 31-30 and 19 are a fixed prefix
//! per identifier, and the meaning is
//!
//! ```text
//! identifier = (raw >> 20) & 0x3FF
//! argument   = (raw >> 8)  & 0x3FF
//! second     =  raw        & 0xFF
//! ```
//!
//! (`studies/itemmods/FINDINGS.md` sections 1, 4.2 and 5.4). [`ModifierWord::id16`] is the first
//! view; [`ModifierWord::identifier`] is the second and is what this module matches on, because a
//! prefix bit must not turn one identifier into another. The 16-bit id is retained for callers
//! that need the exact word key.
//!
//! ## Evidence, per identifier
//!
//! | identifier | effect | evidence |
//! |---|---|---|
//! | 1, 633 | attribute requirement | client code reads 633 and falls back to 1; 570/570 live words have attribute < 51 and rank 1..12 |
//! | 542, 543 | attribute bonus, non-stacking / stacking | client handlers and a live tooltip rendered from a composed 543 |
//! | 555 | enchantment duration | Preservation's `EnchantDuration` builder only; not named by Rurik's table |
//! | 556, 558 | energy, energy recovery | Rurik's tooltip table; 556 is also Preservation's `ExtraEnergy` |
//! | 570 | half skill recharge | Rurik's tooltip table; Preservation's `HalfSkillRecharge` |
//! | 572 | armor rating | Rurik's tooltip table (`Armor: 25`) |
//! | 580 | holds items | Rurik's tooltip table and the backpack's live word |
//! | 584 | damage range, argument is max and second is min | live tooltip `Blunt Dmg: 3-5` from `584 arg 5 arg2 3` |
//! | 587 | damage type | live tooltip `Blunt` from `587 arg 0` |
//! | 527 | armor versus a damage type | Rurik's tooltip table; the second value's role is not measured here, so it is kept as a number |
//! | 576 | the id of an upgrade fitted to the item (`0x2408XXXX`, `XXXX` the upgrade id) | 584 live `CREATE_NAMED_ITEM` records and 173 probed items; `config/World/<pack>/upgrades.json` |
//! | 595 | an upgrade's pair word (`0x2530`/`0x2532`, bit 31 set for an insignia's), low 16 bits = 2 x id (+1) | same; every pair word in the probes names an id word on the same item |
//!
//! The tooltip strings this module builds are **not** character statistics. The client draws an
//! item's tooltip from its modifier words and draws the attribute panel from the server's
//! `AGENT_UPDATE_ATTRIBUTES` column; Rurik's live run showed the panel did not double-apply a
//! word. A future equipment-bonus calculation must read [`ModifierEffect::AttributeBonus`] and
//! add it to that column itself. It must not parse a tooltip line.

/// One raw modifier word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModifierWord(u32);

impl ModifierWord {
    /// The word as the wire carries it.
    pub const fn new(raw: u32) -> Self {
        ModifierWord(raw)
    }

    /// The word as the wire carries it.
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// `raw >> 16`: Headquarter's and the reference client helper's 16-bit id. It includes the
    /// per-identifier prefix bits, so two words with the same semantic identifier can differ here.
    pub const fn id16(self) -> u16 {
        (self.0 >> 16) as u16
    }

    /// `(raw >> 8) & 0xFF`: Headquarter's first argument byte.
    pub const fn arg1(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    /// `raw & 0xFF`: the second value.
    pub const fn arg2(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// `(raw >> 20) & 0x3FF`: the identifier the client dispatches on.
    pub const fn identifier(self) -> u16 {
        ((self.0 >> 20) & 0x3FF) as u16
    }

    /// `(raw >> 8) & 0x3FF`: the 10-bit argument the client handlers read.
    pub const fn argument(self) -> u16 {
        ((self.0 >> 8) & 0x3FF) as u16
    }

    /// Bits 31-30 equal 3, or bit 18 set: the client's walker steps over this word. The word still
    /// travels in the array and is still persisted.
    pub const fn skipped(self) -> bool {
        (self.0 >> 30) == 3 || (self.0 & 0x0004_0000) != 0
    }

    /// Decode the word into its raw value and the meaning the evidence supports.
    pub fn decode(self) -> Modifier {
        let effect = if self.skipped() {
            ModifierEffect::Unknown(self)
        } else {
            self.known_effect()
        };
        Modifier { raw: self, effect }
    }

    fn known_effect(self) -> ModifierEffect {
        let argument = self.argument();
        let second = self.arg2();
        match self.identifier() {
            // 633, the item's attribute requirement, and the legacy identifier 1 the client falls
            // back to. The attribute is the 10-bit argument; the rank is the second byte.
            633 | 1 => ModifierEffect::AttributeRequirement {
                attribute: argument,
                rank: second,
            },
            542 | 543 => ModifierEffect::AttributeBonus {
                attribute: argument,
                amount: second,
                stacking: self.identifier() == 543,
            },
            555 => ModifierEffect::EnchantmentDuration { seconds: second },
            556 => ModifierEffect::Energy { amount: argument },
            558 => ModifierEffect::EnergyRecovery { amount: argument },
            570 => ModifierEffect::HalfSkillRecharge {
                // The chance is the argument: `0x23A81400` is 20%, the word Preservation's
                // `HalfSkillRecharge(20)` builds.
                chance: argument,
            },
            572 => ModifierEffect::Armor { amount: argument },
            580 => ModifierEffect::HoldsItems { count: argument },
            584 => ModifierEffect::DamageRange {
                // `arg` is the maximum and `arg2` the minimum, settled by a live tooltip.
                min: second,
                max: argument,
            },
            587 => ModifierEffect::DamageType {
                // The live `Blunt` line came from `587 arg 0`, so the damage type is the argument.
                damage_type: argument,
            },
            527 => ModifierEffect::ArmorVersus {
                amount: argument,
                damage_type: second,
            },
            // An upgrade fitted to the item: its id is the low 16 bits. The upgrade's own effect words
            // follow its pair word (595); `upgrades.json` says which upgrade the id is.
            576 => ModifierEffect::Upgrade {
                id: (self.0 & 0xFFFF) as u16,
            },
            595 => ModifierEffect::UpgradePair {
                id: ((self.0 & 0xFFFF) >> 1) as u16,
            },
            _ => ModifierEffect::Unknown(self),
        }
    }
}

/// A raw word and what this module can say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifier {
    raw: ModifierWord,
    effect: ModifierEffect,
}

impl Modifier {
    /// The word as it arrived.
    pub const fn word(self) -> ModifierWord {
        self.raw
    }

    /// The word as it arrived, as the `u32` the wire and the database carry.
    pub const fn raw(self) -> u32 {
        self.raw.raw()
    }

    /// The meaning the evidence supports.
    pub const fn effect(self) -> ModifierEffect {
        self.effect
    }

    /// A tooltip line for a known effect. This is presentation only; see the module comment.
    pub fn tooltip(self) -> Option<String> {
        Some(match self.effect {
            ModifierEffect::AttributeRequirement { attribute, rank } => {
                format!("Requires rank {rank} in attribute {attribute}")
            }
            ModifierEffect::AttributeBonus {
                attribute,
                amount,
                stacking,
            } => format!(
                "Attribute {attribute} +{amount} ({})",
                if stacking { "Stacking" } else { "Non-stacking" }
            ),
            ModifierEffect::EnchantmentDuration { seconds } => {
                format!("Enchantments last {seconds} seconds longer")
            }
            ModifierEffect::Energy { amount } => format!("Energy +{amount}"),
            ModifierEffect::EnergyRecovery { amount } => format!("Energy recovery +{amount}"),
            ModifierEffect::HalfSkillRecharge { chance } => {
                format!("{chance}% chance to halve skill recharge")
            }
            ModifierEffect::Armor { amount } => format!("Armor {amount}"),
            ModifierEffect::ArmorVersus {
                amount,
                damage_type,
            } => format!("Armor +{amount} vs. damage type {damage_type}"),
            ModifierEffect::HoldsItems { count } => format!("Holds {count} items"),
            ModifierEffect::DamageRange { min, max } => format!("{min}-{max} damage"),
            ModifierEffect::DamageType { damage_type } => match damage_type {
                0 => "Blunt damage".to_string(),
                other => format!("Damage type {other}"),
            },
            ModifierEffect::Upgrade { id } => format!("Upgrade {id:#x}"),
            ModifierEffect::UpgradePair { .. } => return None,
            ModifierEffect::Unknown(_) => return None,
        })
    }
}

/// What a modifier word means, as far as the sources establish it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierEffect {
    /// `<Attribute> +N`; 543 is the stacking form, 542 the non-stacking one.
    AttributeBonus {
        /// The attribute index (0..50).
        attribute: u16,
        /// The bonus, which is also the rune grade (1 minor, 2 major, 3 superior).
        amount: u8,
        /// True for identifier 543, false for 542.
        stacking: bool,
    },
    /// The item's requirement: the client compares this rank against the character's attribute.
    AttributeRequirement {
        /// The attribute index (0..50; 51 is the client's "no requirement" sentinel).
        attribute: u16,
        /// The rank the character needs.
        rank: u8,
    },
    /// A weapon's damage range; the argument is the maximum and the second value the minimum.
    DamageRange {
        /// The minimum damage.
        min: u8,
        /// The maximum damage.
        max: u16,
    },
    /// The line that names a damage type. 0 is Blunt (measured); other values are not named here.
    DamageType {
        /// The client's damage-type number.
        damage_type: u16,
    },
    /// Extra energy.
    Energy {
        /// The amount.
        amount: u16,
    },
    /// Extra energy recovery.
    EnergyRecovery {
        /// The amount.
        amount: u16,
    },
    /// A chance to halve a skill's recharge.
    HalfSkillRecharge {
        /// The percentage.
        chance: u16,
    },
    /// Extra enchantment duration.
    EnchantmentDuration {
        /// The extra seconds.
        seconds: u8,
    },
    /// An armour rating.
    Armor {
        /// The rating.
        amount: u16,
    },
    /// An armour bonus against one damage type.
    ArmorVersus {
        /// The bonus.
        amount: u16,
        /// The damage type number.
        damage_type: u8,
    },
    /// A container's capacity.
    HoldsItems {
        /// How many items fit.
        count: u16,
    },
    /// An upgrade fitted to the item (insignia, rune or weapon mod), by its id in `upgrades.json`.
    Upgrade {
        /// The upgrade's id.
        id: u16,
    },
    /// An upgrade's pair word: it introduces that upgrade's effect word, which follows it.
    UpgradePair {
        /// The upgrade it belongs to.
        id: u16,
    },
    /// A word whose meaning is not established, including every skipped word.
    Unknown(ModifierWord),
}

/// The ids of the upgrades fitted to an item, in the order its words list them. An item is its base
/// plus these: "Knight's Wyvern Boots of Superior Absorption" is the Wyvern Boots base with upgrades
/// 0x1f9 (Knight's) and 0xfe (Superior Absorption).
pub fn fitted_upgrades(words: &[u32]) -> Vec<u16> {
    decode_all(words)
        .into_iter()
        .filter_map(|modifier| match modifier.effect() {
            ModifierEffect::Upgrade { id } => Some(id),
            _ => None,
        })
        .collect()
}

/// Decode one item's modifier list, preserving every word and its order.
pub fn decode_all(words: &[u32]) -> Vec<Modifier> {
    words
        .iter()
        .map(|raw| ModifierWord::new(*raw).decode())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_views_of_the_word_are_both_available() {
        let word = ModifierWord::new(0xA3C8_5000);
        assert_eq!(word.raw(), 0xA3C8_5000);
        assert_eq!(word.id16(), 0xA3C8, "raw >> 16, prefix included");
        assert_eq!(word.arg1(), 0x50);
        assert_eq!(word.arg2(), 0);
        assert_eq!(word.identifier(), 572, "the client's 10-bit identifier");
        assert_eq!(word.argument(), 80);
        assert!(!word.skipped());
    }

    #[test]
    fn the_backpacks_word_says_it_holds_twenty_items() {
        let word = ModifierWord::new(0x2448_1400);
        assert_eq!(
            word.decode().effect(),
            ModifierEffect::HoldsItems { count: 20 }
        );
        assert_eq!(word.decode().raw(), 0x2448_1400);
        assert_eq!(word.decode().tooltip().as_deref(), Some("Holds 20 items"));
    }

    #[test]
    fn attribute_requirement_uses_the_ten_bit_argument_and_the_rank_byte() {
        let word = ModifierWord::new(0x2798_1509);
        assert_eq!(
            word.decode().effect(),
            ModifierEffect::AttributeRequirement {
                attribute: 21,
                rank: 9
            }
        );
        // Identifier 1 is the client's fallback spelling of the same payload.
        assert_eq!(
            ModifierWord::new(0x0010_1509).decode().effect(),
            ModifierEffect::AttributeRequirement {
                attribute: 21,
                rank: 9
            }
        );
    }

    #[test]
    fn attribute_bonuses_keep_the_stacking_bit_apart() {
        let stacking = ModifierWord::new(0x21F8_1401).decode();
        assert_eq!(
            stacking.effect(),
            ModifierEffect::AttributeBonus {
                attribute: 20,
                amount: 1,
                stacking: true
            }
        );
        assert_eq!(
            stacking.tooltip().as_deref(),
            Some("Attribute 20 +1 (Stacking)")
        );

        let non_stacking = ModifierWord::new(0x21E8_1501).decode();
        assert_eq!(
            non_stacking.effect(),
            ModifierEffect::AttributeBonus {
                attribute: 21,
                amount: 1,
                stacking: false
            }
        );
    }

    #[test]
    fn a_damage_range_reads_the_argument_as_the_maximum() {
        // The live hammer's `584 arg 5 arg2 3` rendered as `Blunt Dmg: 3-5`.
        let word = ModifierWord::new(0xA488_0503).decode();
        assert_eq!(
            word.effect(),
            ModifierEffect::DamageRange { min: 3, max: 5 }
        );
        assert_eq!(word.tooltip().as_deref(), Some("3-5 damage"));
    }

    #[test]
    fn damage_type_zero_is_the_one_name_measured() {
        assert_eq!(
            ModifierWord::new(0x24B8_0000).decode().effect(),
            ModifierEffect::DamageType { damage_type: 0 }
        );
        assert_eq!(
            ModifierWord::new(0x24B8_0000).decode().tooltip().as_deref(),
            Some("Blunt damage")
        );
        assert_eq!(
            ModifierWord::new(0x24B8_0100).decode().tooltip().as_deref(),
            Some("Damage type 1"),
            "no source here names the other damage types"
        );
    }

    #[test]
    fn energy_armour_recharge_and_enchantments_have_their_known_lines() {
        assert_eq!(
            ModifierWord::new(0x62C8_0F00).decode().effect(),
            ModifierEffect::Energy { amount: 15 }
        );
        assert_eq!(
            ModifierWord::new(0x62E8_0100).decode().effect(),
            ModifierEffect::EnergyRecovery { amount: 1 }
        );
        assert_eq!(
            ModifierWord::new(0x23A8_1400).decode().effect(),
            ModifierEffect::HalfSkillRecharge { chance: 20 }
        );
        assert_eq!(
            ModifierWord::new(0x22B8_0014).decode().effect(),
            ModifierEffect::EnchantmentDuration { seconds: 20 }
        );
        assert_eq!(
            ModifierWord::new(0xA0F8_1400).decode().effect(),
            ModifierEffect::ArmorVersus {
                amount: 20,
                damage_type: 0
            }
        );
    }

    #[test]
    fn an_unknown_word_keeps_its_raw_value_and_gets_no_tooltip() {
        let raw = 0x1111_2222;
        assert!(!ModifierWord::new(raw).skipped(), "this is a normal word");
        let decoded = ModifierWord::new(raw).decode();
        assert_eq!(
            decoded.effect(),
            ModifierEffect::Unknown(ModifierWord::new(raw))
        );
        assert_eq!(
            decoded.raw(),
            raw,
            "persistence and serialization keep the word"
        );
        assert_eq!(decoded.tooltip(), None);
    }

    #[test]
    fn a_skipped_word_is_unknown_but_still_present() {
        for raw in [0xC000_0000, 0xC123_4567, 0x0004_0000] {
            let word = ModifierWord::new(raw);
            assert!(word.skipped(), "{raw:#010X}");
            assert_eq!(word.decode().effect(), ModifierEffect::Unknown(word));
            assert_eq!(word.decode().raw(), raw);
        }
    }

    #[test]
    fn upgrade_words_name_the_upgrade_and_its_pair() {
        // Knight's (0x1f9): the id word, and the insignia's high pair word (0x3f2 = 2 x 0x1f9).
        assert_eq!(
            ModifierWord::new(0x2408_01F9).decode().effect(),
            ModifierEffect::Upgrade { id: 0x1F9 }
        );
        assert_eq!(
            ModifierWord::new(0xA530_03F2).decode().effect(),
            ModifierEffect::UpgradePair { id: 0x1F9 }
        );
        // Vitae (0x212): a low pair word whose low bits are 2 x id + 1.
        assert_eq!(
            ModifierWord::new(0x2530_0425).decode().effect(),
            ModifierEffect::UpgradePair { id: 0x212 }
        );
        assert_eq!(
            ModifierWord::new(0x2532_02C6).decode().effect(),
            ModifierEffect::UpgradePair { id: 0x163 }
        );
    }

    #[test]
    fn an_item_is_its_base_plus_its_fitted_upgrades() {
        // "Knight's Wyvern Boots of Superior Absorption", as the probe read it.
        let words = [
            0x2408_01F9, 0x2408_00FE, 0x2530_01FD, 0x27EA_02EA, 0xA3C8_5000, 0x8040_0000,
            0xA0F8_1400, 0xA530_03F2, 0xA7F8_0300,
        ];
        assert_eq!(fitted_upgrades(&words), vec![0x1F9, 0xFE]);
        assert_eq!(fitted_upgrades(&[0xA3C8_3200, 0x8040_0000]), Vec::<u16>::new());
    }

    #[test]
    fn decoding_a_list_keeps_every_word_in_order() {
        let words = [0x24B8_0000, 0xA488_0503, 0x1234_5678];
        let decoded = decode_all(&words);
        assert_eq!(
            decoded
                .iter()
                .map(|modifier| modifier.raw())
                .collect::<Vec<_>>(),
            words
        );
        assert_eq!(
            decoded[2].effect(),
            ModifierEffect::Unknown(ModifierWord::new(0x1234_5678))
        );
    }
}
