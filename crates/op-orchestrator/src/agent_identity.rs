//! Visual identities for the concurrent "agent team" — each parallel
//! design sub-agent gets a distinct colour + name so the canvas can
//! draw per-agent breathing indicators and badges.

/// Fixed 6-colour palette. Colour is assigned by agent index (cycled),
/// so a given team always paints the same colours in the same order.
pub const AGENT_COLORS: [&str; 6] = [
    "#FF6B6B", // coral red
    "#4ECDC4", // teal
    // Cobalt blue — replaced the golden yellow: the name pill renders its
    // label in white, which was unreadable on yellow (user report).
    "#5B8DEF", "#6C5CE7", // purple
    "#51C878", // emerald - replaced pale mint for the same white-label reason
    "#FF8A5C", // warm orange
];

/// Name pool — distinct for the first 12 agents (a team never gets
/// anywhere near that many).
pub const AGENT_NAMES: [&str; 12] = [
    "Kiki", "Mochi", "Pixel", "Nova", "Zuri", "Cleo", "Boba", "Rune", "Fern", "Echo", "Puck",
    "Sage",
];

/// A parallel design agent's visual identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIdentity {
    /// Hex colour string, e.g. `"#FF6B6B"`.
    pub color: String,
    /// Display name, e.g. `"Nova"`.
    pub name: String,
}

/// Assign `count` distinct identities. Colour cycles the palette by
/// index; name is taken from the pool by index (distinct for the first
/// 12 agents).
pub fn assign_agent_identities(count: usize) -> Vec<AgentIdentity> {
    assign_agent_identities_seeded(count, 0)
}

/// Like [`assign_agent_identities`], but rotated by a per-run `seed` so a
/// fresh run meets a fresh face — index 0 was ALWAYS Kiki-in-coral before.
/// Names and colours rotate on co-prime strides (pool sizes 12 and 6), so
/// the same name still shows up in different colours across runs. Teams
/// stay distinct: identities within one call never collide for counts up
/// to the pool sizes.
pub fn assign_agent_identities_seeded(count: usize, seed: u64) -> Vec<AgentIdentity> {
    let name_offset = (seed % AGENT_NAMES.len() as u64) as usize;
    let color_offset = ((seed / AGENT_NAMES.len() as u64) % AGENT_COLORS.len() as u64) as usize;
    (0..count)
        .map(|i| AgentIdentity {
            color: AGENT_COLORS[(color_offset + i) % AGENT_COLORS.len()].to_string(),
            name: AGENT_NAMES[(name_offset + i) % AGENT_NAMES.len()].to_string(),
        })
        .collect()
}

/// Like [`assign_agent_identities`], but the FIRST identity is FIXED to
/// `primary` instead of freshly minted — the remaining `count - 1`
/// identities are drawn from the pool, skipping any colour already used so
/// every group's badge stays visually distinct.
///
/// This is the web-streaming half of the dual-cursor-identity fix
/// (2026-07-17): `web_chat_standard.rs` confirms + announces an identity to
/// the client BEFORE `Orchestrator::run()` even starts (the SSE transcript
/// needs a persona immediately, well before groups/concurrency is known).
/// If the run turns out to be genuinely concurrent, minting a FRESH set of
/// identities (as `assign_agent_identities` does) would silently overwrite
/// that already-announced persona with a different one — the same
/// transcript-vs-cursor split the desktop host hit, just from the opposite
/// direction (the CALLER confirmed first here, instead of the orchestrator).
/// `run.rs`'s `group_identities` computation checks for an already-confirmed
/// `cursor_agent` and, if present, calls this instead of
/// `assign_agent_identities` — so the primary group ADOPTS whatever the
/// caller already told its client, and only the OTHER groups get fresh
/// identities.
pub fn assign_agent_identities_with_primary(
    primary: AgentIdentity,
    count: usize,
) -> Vec<AgentIdentity> {
    if count == 0 {
        return Vec::new();
    }
    let mut out = vec![primary.clone()];
    let mut i: usize = 0;
    // Safety valve: the pools are finite (12 names × 6 colours); stop after
    // a full cycle so a pathological `count` can never loop forever.
    let max_attempts = AGENT_NAMES.len() * AGENT_COLORS.len() + 1;
    while out.len() < count && i < max_attempts {
        let candidate = AgentIdentity {
            color: AGENT_COLORS[i % AGENT_COLORS.len()].to_string(),
            name: AGENT_NAMES[i % AGENT_NAMES.len()].to_string(),
        };
        i += 1;
        if out.iter().any(|existing| existing.color == candidate.color) {
            continue; // keep every group's badge colour distinct
        }
        out.push(candidate);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigns_palette_colors_in_order() {
        let ids = assign_agent_identities(3);
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0].color, "#FF6B6B");
        assert_eq!(ids[1].color, "#4ECDC4");
        assert_eq!(ids[2].color, "#5B8DEF");
        assert_ne!(ids[0].name, ids[1].name);
        assert_ne!(ids[1].name, ids[2].name);
    }

    #[test]
    fn colors_cycle_past_the_palette_size() {
        let ids = assign_agent_identities(7);
        assert_eq!(ids[6].color, ids[0].color);
    }

    #[test]
    fn empty_team_yields_no_identities() {
        assert!(assign_agent_identities(0).is_empty());
    }

    #[test]
    fn with_primary_keeps_the_primary_as_the_first_identity() {
        let primary = AgentIdentity {
            color: "#4ECDC4".into(),
            name: "Nova".into(),
        };
        let ids = assign_agent_identities_with_primary(primary.clone(), 3);
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], primary);
        // The other two never collide with the primary's colour, or each
        // other's.
        assert_ne!(ids[1].color, primary.color);
        assert_ne!(ids[2].color, primary.color);
        assert_ne!(ids[1].color, ids[2].color);
    }

    #[test]
    fn with_primary_count_one_is_just_the_primary() {
        let primary = AgentIdentity {
            color: "#FF8A5C".into(),
            name: "Puck".into(),
        };
        let ids = assign_agent_identities_with_primary(primary.clone(), 1);
        assert_eq!(ids, vec![primary]);
    }

    #[test]
    fn with_primary_zero_count_yields_nothing() {
        let primary = AgentIdentity {
            color: "#FF6B6B".into(),
            name: "Kiki".into(),
        };
        assert!(assign_agent_identities_with_primary(primary, 0).is_empty());
    }

    #[test]
    fn with_primary_that_collides_with_pool_index_zero_still_stays_distinct() {
        // Primary happens to be exactly what `assign_agent_identities` would
        // have picked for index 0 ("Kiki"/coral) — the walk must skip that
        // slot for the OTHER members instead of duplicating it.
        let primary = AgentIdentity {
            color: "#FF6B6B".into(),
            name: "Kiki".into(),
        };
        let ids = assign_agent_identities_with_primary(primary.clone(), 4);
        assert_eq!(ids[0], primary);
        for i in 1..ids.len() {
            assert_ne!(
                ids[i].color, primary.color,
                "index {i} collided with primary"
            );
            for j in (i + 1)..ids.len() {
                assert_ne!(ids[i].color, ids[j].color, "index {i} and {j} collided");
            }
        }
    }

    #[test]
    fn seed_rotates_names_and_colors_but_keeps_teams_distinct() {
        let a = assign_agent_identities_seeded(3, 0);
        let b = assign_agent_identities_seeded(3, 5);
        assert_ne!(a[0].name, b[0].name, "a fresh seed meets a fresh face");
        let c = assign_agent_identities_seeded(4, 17);
        for i in 0..c.len() {
            for j in (i + 1)..c.len() {
                assert_ne!(c[i].name, c[j].name, "teammates stay distinct");
                assert_ne!(c[i].color, c[j].color);
            }
        }
    }
}
