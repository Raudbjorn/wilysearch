//! Default TTRPG (Tabletop Role-Playing Game) synonym definitions.
//!
//! Provides a comprehensive synonym map covering common D&D 5e terminology
//! including stat abbreviations, game mechanics, conditions, book abbreviations,
//! damage types, creature types, and player slang.

use super::super::synonyms::{SynonymConfig, SynonymMap};

/// Build a comprehensive TTRPG synonym map with default game terms.
///
/// Includes:
/// - Stat abbreviations (str, dex, con, int, wis, cha, hp, ac, dc, cr, xp)
/// - Game mechanics (aoo, crit, nat 20, tpk, dm, gm, pc, npc, bbeg)
/// - Condition terms (prone, grappled, stunned, frightened, poisoned, etc.)
/// - Book abbreviations (phb, dmg, mm, xge, tce)
/// - Damage types (fire, cold, lightning, necrotic, radiant, psychic)
/// - Creature types (undead, dragon, devil, fiend)
pub fn build_default_ttrpg_synonyms() -> SynonymMap {
    let mut map = SynonymMap::with_config(SynonymConfig {
        max_expansions: 5,
        enabled: true,
        include_original: true,
    });

    // === Stat abbreviations (multi-way) ===
    map.add_multi_way(&["hp", "hit points", "health", "life"]);
    map.add_multi_way(&["ac", "armor class", "armour class"]);
    map.add_multi_way(&["str", "strength"]);
    map.add_multi_way(&["dex", "dexterity"]);
    map.add_multi_way(&["con", "constitution"]);
    map.add_multi_way(&["int", "intelligence"]);
    map.add_multi_way(&["wis", "wisdom"]);
    map.add_multi_way(&["cha", "charisma"]);
    map.add_multi_way(&["dc", "difficulty class"]);
    map.add_multi_way(&["cr", "challenge rating"]);
    map.add_multi_way(&["xp", "experience points", "exp"]);
    map.add_multi_way(&["pp", "passive perception"]);

    // === Game mechanics ===
    map.add_multi_way(&["aoo", "opportunity attack", "attack of opportunity"]);
    map.add_multi_way(&["tpk", "total party kill"]);
    map.add_multi_way(&["nat 20", "natural 20", "critical hit", "crit"]);
    map.add_multi_way(&["nat 1", "natural 1", "critical failure", "fumble"]);
    map.add_multi_way(&["dm", "dungeon master", "game master", "gm"]);
    map.add_multi_way(&["pc", "player character"]);
    map.add_multi_way(&["npc", "non-player character"]);
    map.add_multi_way(&["bbeg", "big bad evil guy", "main villain"]);

    // === Condition synonyms ===
    map.add_multi_way(&["prone", "knocked down", "on the ground"]);
    map.add_multi_way(&["grappled", "grabbed", "held"]);
    map.add_multi_way(&["stunned", "stun", "dazed"]);
    map.add_multi_way(&["frightened", "scared", "afraid", "fear"]);
    map.add_multi_way(&["poisoned", "poison", "toxic"]);
    map.add_multi_way(&["invisible", "invisibility", "unseen"]);
    map.add_multi_way(&["blinded", "blind"]);
    map.add_multi_way(&["deafened", "deaf"]);
    map.add_multi_way(&["charmed", "charm"]);
    map.add_multi_way(&["paralyzed", "paralysis"]);
    map.add_multi_way(&["petrified", "turned to stone"]);
    map.add_multi_way(&["restrained", "restrain"]);
    map.add_multi_way(&["incapacitated", "incapacitate"]);
    map.add_multi_way(&["unconscious", "knocked out", "ko"]);
    map.add_multi_way(&["exhaustion", "exhausted", "fatigue"]);

    // === Book / source abbreviations ===
    map.add_multi_way(&["phb", "player's handbook", "players handbook"]);
    map.add_multi_way(&["dmg", "dungeon master's guide", "dungeon masters guide"]);
    map.add_multi_way(&["mm", "monster manual"]);
    map.add_multi_way(&["xge", "xanathar's guide", "xanathars guide to everything"]);
    map.add_multi_way(&["tce", "tasha's cauldron", "tashas cauldron of everything"]);
    map.add_multi_way(&["vgm", "volo's guide", "volos guide to monsters"]);
    map.add_multi_way(&["mtof", "mordenkainen's tome", "mordenkainens tome of foes"]);
    map.add_multi_way(&["scag", "sword coast adventurers guide"]);

    // === Damage types (one-way - "fire" shouldn't expand to "fire damage") ===
    map.add_one_way("fire damage", &["flame damage", "burning"]);
    map.add_one_way("cold damage", &["frost damage", "ice damage", "freezing"]);
    map.add_one_way("lightning damage", &["electric damage", "shock damage"]);
    map.add_one_way("thunder damage", &["sonic damage"]);
    map.add_one_way("necrotic damage", &["death damage", "dark damage"]);
    map.add_one_way("radiant damage", &["holy damage", "light damage"]);
    map.add_one_way("psychic damage", &["mental damage", "mind damage"]);
    map.add_one_way("force damage", &["magical damage"]);
    map.add_one_way("poison damage", &["toxic damage"]);
    map.add_one_way("acid damage", &["corrosive damage"]);

    // === Creature types (one-way hierarchies) ===
    map.add_one_way("dragon", &["wyrm", "drake", "wyvern"]);
    map.add_one_way("devil", &["fiend", "demon", "daemon"]);
    map.add_one_way("undead", &["zombie", "skeleton", "vampire", "lich", "ghost", "wraith"]);
    map.add_one_way("giant", &["ogre", "troll", "cyclops"]);
    map.add_one_way("goblinoid", &["goblin", "hobgoblin", "bugbear"]);

    // === Action types ===
    map.add_multi_way(&["bonus action", "ba"]);
    map.add_multi_way(&["reaction", "rxn"]);
    map.add_multi_way(&["concentration", "conc"]);
    map.add_multi_way(&["advantage", "adv"]);
    map.add_multi_way(&["disadvantage", "disadv", "dis"]);

    // === Spell components ===
    map.add_multi_way(&["verbal", "v"]);
    map.add_multi_way(&["somatic", "s"]);
    map.add_multi_way(&["material", "m"]);

    // === Common misnomers / player slang ===
    map.add_one_way("healing word", &["cure wounds"]); // players often confuse
    map.add_one_way("magic missile", &["magic missle"]); // common misspelling as synonym

    map
}
