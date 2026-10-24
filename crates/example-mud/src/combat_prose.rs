//! Presentation only: resolved attacks in, prose out; never consumes dice.
use crate::savage_rules::{AttackOutcome, AttackProfile, DamageType, SoulData};

pub(crate) struct Strike<'a> {
    pub player: bool,
    pub name: &'a str,
    pub weapon: &'a str,
    pub profile: AttackProfile,
    pub trample: bool,
    pub counter: bool,
    pub variant: usize,
}

pub(crate) fn landed(outcome: &AttackOutcome, defender: &SoulData, strike: Strike<'_>) -> String {
    let (actor, target) = if strike.player {
        ("You".to_owned(), strike.name)
    } else {
        let mut chars = strike.name.chars();
        let name = chars
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_else(|| "Opponent".into());
        (name, "you")
    };
    let verb = |first: &str, third: &str| {
        if strike.player {
            first.to_owned()
        } else {
            third.to_owned()
        }
    };
    let action = if strike.trample {
        "trample".to_owned()
    } else if strike.counter {
        "counterattacks".to_owned()
    } else if strike.profile.ranged {
        verb("shoot", "shoots")
    } else {
        match strike.profile.damage_type {
            DamageType::Energy => verb("blast", "blasts"),
            DamageType::Sharp => match strike.variant % 3 {
                0 => verb("slash", "slashes"),
                1 => verb("cut", "cuts"),
                _ => verb("strike", "strikes"),
            },
            DamageType::Blunt => match strike.variant % 3 {
                0 => verb("strike", "strikes"),
                1 if strike.player && strike.weapon.is_empty() => "punch".into(),
                1 => verb("hit", "hits"),
                _ => verb("land a blow on", "lands a blow on"),
            },
        }
    };
    let weapon = if strike.weapon.is_empty() || strike.trample {
        String::new()
    } else {
        format!(" with {}", grim::color::escape_codes(strike.weapon))
    };
    let contact = format!("{actor} {action} {target}{weapon}");
    if outcome.wound_level <= 0 {
        return match strike.variant % 3 {
            0 => format!("{contact}, but the hit causes no wound."),
            1 => format!("{contact}; the blow fails to wound."),
            _ => format!("{contact}, making contact without a wound."),
        };
    }
    let force = if outcome.wound_level >= 4 {
        " with a crushing blow"
    } else {
        ""
    };
    // 'Crushing' describes blunt impact only; sharp/energy retain their own verb.
    let force = if matches!(strike.profile.damage_type, DamageType::Blunt) && !strike.trample {
        force
    } else {
        ""
    };
    format!(
        "{contact}{force}: {}{}.",
        defender.injury.label().to_ascii_lowercase(),
        if defender.stunned && !defender.injury.is_dead() {
            ", stunned"
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn result(wound: i32) -> AttackOutcome {
        AttackOutcome {
            attack_total: 15,
            defense_total: 10,
            success_margin: 5,
            total_damage: 5,
            wound_level: wound,
            hit: true,
            defense_used: false,
            injury_before: Default::default(),
            injury_after: Default::default(),
            stunned: false,
            unconscious: false,
        }
    }
    #[test]
    fn contact_without_wound_never_claims_an_injury_or_stun() {
        let soul = SoulData::default();
        for variant in 0..3 {
            let line = landed(
                &result(0),
                &soul,
                Strike {
                    player: true,
                    name: "artrell",
                    weapon: "",
                    profile: AttackProfile::unarmed(&soul),
                    trample: false,
                    counter: false,
                    variant,
                },
            );
            assert!(line.contains("wound"));
            assert!(!line.contains("stunned") && !line.contains("injured"));
        }
    }
    #[test]
    fn sharp_strike_uses_equipped_weapon_and_actual_condition() {
        let soul = SoulData::default();
        let mut profile = AttackProfile::unarmed(&soul);
        profile.damage_type = DamageType::Sharp;
        let line = landed(
            &result(4),
            &soul,
            Strike {
                player: true,
                name: "artrell",
                weapon: "Light Sword",
                profile,
                trample: false,
                counter: false,
                variant: 0,
            },
        );
        assert!(line.starts_with("You slash artrell with Light Sword:"));
        assert!(!line.contains("crushing"));
    }
}
