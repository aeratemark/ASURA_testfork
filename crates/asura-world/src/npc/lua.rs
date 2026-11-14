//! The Lua behavior source: content declares a tree, Rust builds and ticks it.
//!
//! `docs/adr/0014-behavior-tree-execution.md` fixes the split: the runtime, node semantics and
//! running state are Rust's; the tree's shape is a content script. A table entry is a tree spec; a
//! function entry is a script leaf, called once per agent per tick with a Rust-owned `ctx.state` and a
//! command table (docs/59). The VM stays alive for the script leaves, and mlua's `send` feature makes
//! the source shareable across the world's instance tasks.
//!
//! The script is `config/World/<pack>/npcs/behaviors.lua` and returns a table keyed by the behavior
//! name a placement uses:
//!
//! ```lua
//! return {
//!   patrol = { kind = "patrol" },
//!   worm_lakeside = {
//!     burrow = true,
//!     script = function(ctx)
//!       if ctx.in_combat and ctx.phase == "hidden" then ctx:emerge() end
//!       if not ctx.in_combat and ctx.phase == "out" then ctx:burrow(4000) end
//!       return "running"
//!     end,
//!   },
//! }
//! ```
//!
//! The supported node kinds are the runtime's own; a name the script does not return is an unknown
//! behavior and refuses the pack at load, exactly as an unknown `patrol` does today.

use super::behavior::{Script, Spec};
use super::table::BehaviorSource;
use mlua::{Function, Lua, LuaOptions, MultiValue, StdLib, Table, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// The script inside a pack that declares behavior trees.
pub const BEHAVIORS_FILE: &str = "npcs/behaviors.lua";

/// The behavior trees a pack's Lua script declared, by the name a placement uses. The VM stays alive:
/// a function entry is called once per agent per tick, and its state is a table this source creates.
pub struct LuaBehaviors {
    lua: Lua,
    specs: BTreeMap<String, Spec>,
    functions: BTreeMap<String, Function>,
}

impl LuaBehaviors {
    /// Run `path` once and read the table it returns. Every problem is a string naming the file and
    /// the entry, so the pack check can report all of them.
    pub fn load(path: &Path) -> Result<LuaBehaviors, String> {
        let source = std::fs::read_to_string(path)
            .map_err(|err| format!("{}: {err}", path.display()))?;
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH,
            LuaOptions::default(),
        )
        .map_err(|err| format!("{}: {err}", path.display()))?;
        let table: Table = lua
            .load(&source)
            .set_name(path.to_string_lossy())
            .eval()
            .map_err(|err| format!("{}: {err}", path.display()))?;
        let mut specs = BTreeMap::new();
        let mut functions = BTreeMap::new();
        for pair in table.pairs::<String, Value>() {
            let (name, value) = pair.map_err(|err| format!("{}: {err}", path.display()))?;
            let spec = match value {
                Value::Function(function) => {
                    functions.insert(name.clone(), function);
                    Spec::Script {
                        name: name.clone(),
                        burrow: false,
                    }
                }
                Value::Table(table) => {
                    if let Ok(Some(function)) = table.get::<Option<Function>>("script") {
                        let burrow = table
                            .get::<Option<bool>>("burrow")
                            .map_err(|err| format!("{name}.burrow: {err}"))?
                            .unwrap_or(false);
                        functions.insert(name.clone(), function);
                        Spec::Script {
                            name: name.clone(),
                            burrow,
                        }
                    } else {
                        parse_spec(&table).map_err(|err| format!("{name}: {err}"))?
                    }
                }
                other => {
                    return Err(format!(
                        "{name}: expected a table or a function, found {}",
                        other.type_name()
                    ))
                }
            };
            if specs.insert(name.clone(), spec).is_some() {
                return Err(format!("{name}: declared more than once"));
            }
        }
        if specs.is_empty() {
            return Err(format!("{}: no behaviors", path.display()));
        }
        Ok(LuaBehaviors {
            lua,
            specs,
            functions,
        })
    }

    /// The entries this source declares, for the start-up line and for tests.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.specs.keys().map(String::as_str)
    }
}

impl BehaviorSource for LuaBehaviors {
    fn spec(&self, name: &str) -> Option<Spec> {
        self.specs.get(name).cloned()
    }

    fn script(&self, name: &str) -> Option<Script> {
        let function = self.functions.get(name)?.clone();
        let state = self.lua.create_table().ok()?;
        let commands = self.lua.create_table().ok()?;
        let context = self.lua.create_table().ok()?;
        context.set("state", state.clone()).ok()?;
        let burrow = commands.clone();
        context
            .set(
                "burrow",
                self.lua
                    .create_function(move |lua, args: MultiValue| {
                        let command = lua.create_table()?;
                        command.set("op", "burrow")?;
                        if let Some(hidden_ms) = args
                            .get(1)
                            .and_then(Value::as_integer)
                            .filter(|ms| *ms >= 0)
                        {
                            command.set("hidden_ms", hidden_ms as u32)?;
                        }
                        burrow.raw_set(burrow.raw_len() + 1, command)
                    })
                    .ok()?,
            )
            .ok()?;
        let emerge = commands.clone();
        context
            .set(
                "emerge",
                self.lua
                    .create_function(move |lua, _: MultiValue| {
                        let command = lua.create_table()?;
                        command.set("op", "emerge")?;
                        emerge.raw_set(emerge.raw_len() + 1, command)
                    })
                    .ok()?,
            )
            .ok()?;
        Some(Script {
            function,
            context,
            state,
            commands,
        })
    }
}

/// The pack's script, or `None` when the pack has none and the built-in `patrol` is all it needs.
pub fn source(pack: &Path) -> Result<Option<LuaBehaviors>, String> {
    let path = pack.join(BEHAVIORS_FILE);
    if !path.exists() {
        return Ok(None);
    }
    LuaBehaviors::load(&path).map(Some)
}

fn parse_spec(table: &Table) -> Result<Spec, String> {
    let kind: String = table
        .get("kind")
        .map_err(|err| format!("kind: {err}"))?;
    match kind.as_str() {
        "sequence" => {
            keys(table, &["kind", "children"])?;
            Ok(Spec::Sequence(children(table)?))
        }
        "fallback" => {
            keys(table, &["kind", "children"])?;
            Ok(Spec::Fallback(children(table)?))
        }
        "wait" => {
            keys(table, &["kind", "ms"])?;
            Ok(Spec::Wait {
                ms: table.get("ms").map_err(|err| format!("ms: {err}"))?,
            })
        }
        "stand" => {
            keys(table, &["kind"])?;
            Ok(Spec::Stand)
        }
        "patrol" => {
            keys(table, &["kind", "route"])?;
            let route: Option<String> = table
                .get("route")
                .map_err(|err| format!("route: {err}"))?;
            Ok(Spec::Patrol { route })
        }
        "burrow" => {
            keys(table, &["kind", "visible_ms", "hidden_ms"])?;
            Ok(Spec::Burrow {
                visible_ms: table
                    .get("visible_ms")
                    .map_err(|err| format!("visible_ms: {err}"))?,
                hidden_ms: table
                    .get("hidden_ms")
                    .map_err(|err| format!("hidden_ms: {err}"))?,
            })
        }
        other => Err(format!(
            "unknown node kind {other:?}; try sequence, fallback, wait, stand, patrol, burrow"
        )),
    }
}

fn children(table: &Table) -> Result<Vec<Spec>, String> {
    let list: Table = table
        .get("children")
        .map_err(|err| format!("children: {err}"))?;
    let mut out = Vec::new();
    for (index, child) in list.sequence_values::<Table>().enumerate() {
        let child = child.map_err(|err| format!("children[{index}]: {err}"))?;
        out.push(parse_spec(&child).map_err(|err| format!("children[{index}]: {err}"))?);
    }
    if out.is_empty() {
        return Err("children: a control node needs at least one child".to_string());
    }
    Ok(out)
}

fn keys(table: &Table, allowed: &[&str]) -> Result<(), String> {
    for pair in table.pairs::<Value, Value>() {
        let (key, _) = pair.map_err(|err| format!("keys: {err}"))?;
        let Value::String(key) = key else {
            return Err("every key is a string".to_string());
        };
        let key = key.to_str().map_err(|err| format!("key: {err}"))?;
        if !allowed.contains(&key.as_ref()) {
            return Err(format!(
                "unknown field {key:?}; this node accepts {}",
                allowed.join(", ")
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<LuaBehaviors, String> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH,
            LuaOptions::default(),
        )
        .expect("lua");
        let table: Table = lua.load(text).eval().expect("table");
        let mut specs = BTreeMap::new();
        let mut functions = BTreeMap::new();
        for pair in table.pairs::<String, Value>() {
            let (name, value) = pair.expect("pair");
            match value {
                Value::Function(function) => {
                    functions.insert(name.clone(), function);
                    specs.insert(
                        name.clone(),
                        Spec::Script {
                            name,
                            burrow: false,
                        },
                    );
                }
                Value::Table(table) => {
                    if let Some(function) = table.get::<Option<Function>>("script").expect("script") {
                        let burrow = table
                            .get::<Option<bool>>("burrow")
                            .expect("burrow")
                            .unwrap_or(false);
                        functions.insert(name.clone(), function);
                        specs.insert(
                            name.clone(),
                            Spec::Script {
                                name,
                                burrow,
                            },
                        );
                    } else {
                        specs.insert(name, parse_spec(&table).expect("spec"));
                    }
                }
                other => panic!("{name}: unexpected {}", other.type_name()),
            }
        }
        Ok(LuaBehaviors {
            lua,
            specs,
            functions,
        })
    }

    #[test]
    fn the_committed_script_declares_the_worm_cycle() {
        let pack = crate::spawns::pack();
        let source = source(&pack)
            .expect("the committed script loads")
            .expect("the pack has a behavior script");
        assert_eq!(
            source.spec("worm_lakeside"),
            Some(Spec::Script {
                name: "worm_lakeside".to_string(),
                burrow: true,
            })
        );
        assert!(source.script("worm_lakeside").is_some());
        assert_eq!(
            source.spec("patrol"),
            Some(Spec::Patrol { route: None })
        );
    }

    #[test]
    fn the_committed_worm_stays_up_in_combat_and_burrows_for_a_drawn_span() {
        let pack = crate::spawns::pack();
        let source = source(&pack)
            .expect("the committed script loads")
            .expect("the pack has a behavior script");
        let script = source.script("worm_lakeside").expect("the worm script");
        let run = |script: &Script, in_combat: bool, phase: &str, delta_ms: u32| {
            script.context.set("in_combat", in_combat).expect("combat");
            script.context.set("phase", phase).expect("phase");
            script.context.set("delta_ms", delta_ms).expect("delta");
            script
                .function
                .call::<Option<String>>(script.context.clone())
                .expect("the committed script runs");
        };

        // In a fight, a surfaced worm never asks to submerge.
        run(&script, true, "out", 60_000);
        assert_eq!(script.commands.raw_len(), 0, "in combat it stays up");

        // Out of a fight, it stays up for a drawn 5-10 s span, then asks to submerge for a drawn
        // 5-15 s span (`docs/59`).
        let mut burrow = None;
        for second in 1..=10u32 {
            run(&script, false, "out", 1_000);
            if script.commands.raw_len() != 0 {
                burrow = Some(second);
                break;
            }
        }
        let second = burrow.expect("a worm out of combat submerges within 10 s");
        assert!(5 <= second, "the surface span is at least 5 s: {second}");
        let command: Table = script.commands.get(1).expect("the burrow command");
        assert_eq!(command.get::<String>("op").expect("op"), "burrow");
        let hidden = command.get::<u32>("hidden_ms").expect("hidden_ms");
        assert!(
            (5_000..=15_000).contains(&hidden),
            "the hidden span is drawn from 5-15 s, got {hidden}"
        );
    }

    #[test]
    fn the_committed_worm_queen_emerges_once_and_never_asks_to_burrow() {
        let pack = crate::spawns::pack();
        let source = source(&pack)
            .expect("the committed script loads")
            .expect("the pack has a behavior script");
        assert_eq!(
            source.spec("worm_queen"),
            Some(Spec::Script {
                name: "worm_queen".to_string(),
                burrow: true,
            })
        );
        let script = source.script("worm_queen").expect("the queen script");
        script.context.set("phase", "out").expect("phase");
        for _ in 0..60 {
            script.context.set("in_combat", false).expect("combat");
            script.context.set("delta_ms", 1_000).expect("delta");
            script
                .function
                .call::<Option<String>>(script.context.clone())
                .expect("the queen script runs");
        }
        assert_eq!(
            script.commands.raw_len(),
            0,
            "the queen comes up once through the burrow flag and never submerges"
        );
    }

    #[test]
    fn a_script_context_writes_the_burrow_command() {
        let source = parse(
            r#"
            local function worm(ctx) return "running" end
            return { worm = { script = worm, burrow = true } }
            "#,
        )
        .expect("parses");
        let script = source.script("worm").expect("script");
        assert_eq!(
            script
                .function
                .call::<Option<String>>(script.context.clone())
                .expect("call"),
            Some("running".to_string())
        );
        let burrow: Function = script.context.get("burrow").expect("burrow");
        burrow
            .call::<()>((script.context.clone(), 4000u32))
            .expect("ctx:burrow(4000)");
        let command: Table = script
            .commands
            .get(1)
            .expect("the command table");
        assert_eq!(
            command.get::<String>("op").expect("op"),
            "burrow".to_string()
        );
        assert_eq!(command.get::<u32>("hidden_ms").expect("hidden_ms"), 4000);
    }

    #[test]
    fn every_node_kind_round_trips() {
        let source = parse(
            r#"
            return {
              one = { kind = "sequence", children = {
                { kind = "wait", ms = 250 },
                { kind = "stand" },
                { kind = "patrol", route = "square" },
                { kind = "burrow", visible_ms = 1, hidden_ms = 2 },
              } },
              two = { kind = "fallback", children = { { kind = "stand" } } },
            }
            "#,
        )
        .expect("parses");
        assert_eq!(
            source.spec("one"),
            Some(Spec::Sequence(vec![
                Spec::Wait { ms: 250 },
                Spec::Stand,
                Spec::Patrol {
                    route: Some("square".to_string())
                },
                Spec::Burrow {
                    visible_ms: 1,
                    hidden_ms: 2
                },
            ]))
        );
        assert_eq!(
            source.spec("two"),
            Some(Spec::Fallback(vec![Spec::Stand]))
        );
    }

    #[test]
    fn an_unknown_field_is_named() {
        let lua = Lua::new();
        let table: Table = lua
            .load(r#"return { kind = "stand", speeed = 1 }"#)
            .eval()
            .expect("table");
        let err = parse_spec(&table).expect_err("unknown field");
        assert!(err.contains("speeed"), "{err}");
    }
}
