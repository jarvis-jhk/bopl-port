//! Scripted scenarios for golden comparisons with the original
//! (`docs/review/feel-pass.md`, `scripts/dev/golden/`): a small text format
//! that compiles to a `.rep` replay the original's viewer plays.
//!
//! ```text
//! # comment
//! level 6                 # build index (Level1 = 6)
//! seed 12345              # sim RNG seed of the round
//! abilities 3             # ability slots
//! player color=1 team=0 abilities=4,7,2      # player 1 (list order = ids 1..4)
//! player color=4 team=1 abilities=2 kbm      # kbm: keyboard + mouse (WASD moves)
//! script 1 230: ; 12:R ; 3:UJ ; 40:          # TICKS:KEYS steps, from tick 0
//! script 2 300:^135 1 ; 5:^135              # ^DEG aim angle, @B raw angle byte
//! ticks 900               # length (neutral input after the scripts)
//! ```
//!
//! Keys: `L R U D` direction (summed, then the angle byte), `^DEG` direction
//! by angle in degrees (0 = right, 90 = up), `@B` raw angle byte, `J` jump,
//! `1 2 3` ability buttons, `w a s d` keys (keyboard players). `script` lines
//! for the same player append.

use anyhow::{Context, Result, bail};
use bopl_sim::fix::Fix;
use bopl_sim::input::{
    BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, KEY_A, KEY_D, KEY_S, KEY_W, PlayerInput,
    angle_byte_from_direction,
};
use bopl_sim::vec2::Vec2;

use crate::replay::{Replay, ReplayPlayer};

/// A parsed scenario.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub level: usize,
    pub seed: u32,
    pub abilities: u8,
    pub players: Vec<ReplayPlayer>,
    /// Per player (list order), per tick.
    pub scripts: Vec<Vec<PlayerInput>>,
    pub ticks: usize,
}

/// Angle byte of a direction given in degrees (the original's encoding:
/// byte `b` means `2π (b − 1) / 255`).
pub fn angle_byte_from_degrees(deg: f64) -> u8 {
    let b = (deg.rem_euclid(360.0) * 255.0 / 360.0).round() as i64 % 255;
    (b + 1) as u8
}

fn parse_keys(k: &str) -> Result<PlayerInput> {
    let mut d = Vec2::ZERO;
    let mut buttons = 0u8;
    let mut keys = 0u8;
    let mut angle: Option<u8> = None;
    let cs: Vec<char> = k.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        match c {
            'L' => d.x -= Fix::ONE,
            'R' => d.x += Fix::ONE,
            'U' => d.y += Fix::ONE,
            'D' => d.y -= Fix::ONE,
            'J' => buttons |= BTN_JUMP,
            '1' => buttons |= BTN_ABILITY0,
            '2' => buttons |= BTN_ABILITY1,
            '3' => buttons |= BTN_ABILITY2,
            'w' => keys |= KEY_W,
            'a' => keys |= KEY_A,
            's' => keys |= KEY_S,
            'd' => keys |= KEY_D,
            '^' | '@' => {
                let st = i;
                while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.' || cs[i] == '-') {
                    i += 1;
                }
                let n: String = cs[st..i].iter().collect();
                angle = Some(if c == '^' {
                    angle_byte_from_degrees(n.parse().with_context(|| format!("bad angle {k}"))?)
                } else {
                    n.parse().with_context(|| format!("bad angle byte {k}"))?
                });
            }
            ' ' => {}
            _ => bail!("unknown key {c:?} in {k:?}"),
        }
    }
    let mut p = PlayerInput::new(
        angle.unwrap_or_else(|| angle_byte_from_direction(d)),
        buttons,
    );
    p.keys = keys;
    Ok(p)
}

/// Expands `TICKS:KEYS ; ...` steps into per-tick inputs.
pub fn expand(s: &str) -> Result<Vec<PlayerInput>> {
    let mut out = Vec::new();
    for step in s.split(';').filter(|x| !x.trim().is_empty()) {
        let (n, k) = step.split_once(':').unwrap_or((step, ""));
        let n: usize = n
            .trim()
            .parse()
            .with_context(|| format!("bad step {step:?}"))?;
        out.extend(std::iter::repeat_n(parse_keys(k.trim())?, n));
    }
    Ok(out)
}

impl Scenario {
    pub fn parse(text: &str) -> Result<Scenario> {
        let mut sc = Scenario {
            level: 6,
            seed: 1,
            abilities: 3,
            players: Vec::new(),
            scripts: Vec::new(),
            ticks: 0,
        };
        for (ln, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let rest = rest.trim();
            let ctx = || format!("line {}: {line}", ln + 1);
            match cmd {
                "level" => sc.level = rest.parse().with_context(ctx)?,
                "seed" => sc.seed = rest.parse().with_context(ctx)?,
                "abilities" => sc.abilities = rest.parse().with_context(ctx)?,
                "ticks" => sc.ticks = rest.parse().with_context(ctx)?,
                "player" => {
                    let id = sc.players.len() as i32 + 1;
                    if id > 4 {
                        bail!("{}: at most 4 players in a replay", ctx());
                    }
                    let mut p = ReplayPlayer {
                        id,
                        uses_keyboard_and_mouse: false,
                        color: id as u8,
                        team: (id - 1) as u8,
                        abilities: [2, 0, 0],
                    };
                    for kv in rest.split_whitespace() {
                        match kv.split_once('=') {
                            Some(("color", v)) => p.color = v.parse().with_context(ctx)?,
                            Some(("team", v)) => p.team = v.parse().with_context(ctx)?,
                            Some(("abilities", v)) => {
                                p.abilities = [0; 3];
                                for (i, a) in v.split(',').enumerate().take(3) {
                                    p.abilities[i] = a.parse().with_context(ctx)?;
                                }
                            }
                            None if kv == "kbm" => p.uses_keyboard_and_mouse = true,
                            _ => bail!("{}: unknown player field {kv}", ctx()),
                        }
                    }
                    if p.abilities[0] == 0 {
                        bail!("{}: ability 1 must be non-zero (marks presence)", ctx());
                    }
                    sc.players.push(p);
                    sc.scripts.push(Vec::new());
                }
                "script" => {
                    let (who, steps) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                    let who: usize = who.parse().with_context(ctx)?;
                    let s = sc
                        .scripts
                        .get_mut(who.wrapping_sub(1))
                        .with_context(|| format!("{}: no player {who}", ctx()))?;
                    s.extend(expand(steps).with_context(ctx)?);
                }
                _ => bail!("{}: unknown command", ctx()),
            }
        }
        if sc.players.is_empty() {
            bail!("no players");
        }
        let longest = sc.scripts.iter().map(Vec::len).max().unwrap_or(0);
        sc.ticks = sc.ticks.max(longest);
        Ok(sc)
    }

    /// The replay the original plays (local-game header conventions:
    /// delay 0, demo mask 255).
    pub fn replay(&self) -> Replay {
        let mut frames = vec![[PlayerInput::default(); 4]; self.ticks];
        for (i, s) in self.scripts.iter().enumerate() {
            let slot = (self.players[i].id - 1) as usize;
            for (t, inp) in s.iter().enumerate().take(self.ticks) {
                frames[t][slot] = *inp;
            }
        }
        Replay {
            sequence: 0,
            seed: self.seed,
            number_of_players: self.players.len() as u8,
            number_of_abilities: self.abilities,
            level: (self.level - 6) as u8,
            delay: 0,
            demo_mask: 255,
            players: self.players.clone(),
            frames,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_round_trips() {
        let sc = Scenario::parse(
            "level 7\nseed 9\nplayer color=4 abilities=4,7\nplayer kbm team=3\n\
             script 1 10: ; 2:RJ ; 1:^90 1\nscript 2 3:wd\nticks 40\n",
        )
        .unwrap();
        assert_eq!(sc.ticks, 40);
        assert_eq!(sc.players[1].team, 3);
        assert!(sc.players[1].uses_keyboard_and_mouse);
        let r = Replay::parse(&sc.replay().write()).unwrap();
        assert_eq!(r.level, 1);
        assert_eq!(r.players, sc.players);
        assert_eq!(r.frames[10][0].buttons, BTN_JUMP);
        assert_eq!(r.frames[12][0].angle, angle_byte_from_degrees(90.0));
        assert_eq!(r.frames[12][0].buttons, BTN_ABILITY0);
        assert_eq!(r.frames[1][1].keys, KEY_W | KEY_D);
        assert_eq!(angle_byte_from_degrees(0.0), 1);
        assert_eq!(angle_byte_from_degrees(180.0), 129);
    }
}
