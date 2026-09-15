//! One-sided race evaluation: how many rolls `x` still needs to finish.
//!
//! In a race neither side can touch the other, so "how fast can I finish" is a
//! purely one-sided question. That matters because the probability-based value the
//! nets produce is *flat* once a race is decided: a four-roll win and a seven-roll
//! win are both worth exactly one point, so every legal move evaluates the same and
//! the choice between them falls to an arbitrary tie-break. Ordering those ties by
//! expected rolls is what makes the engine actually finish a won race promptly.
//!
//! Why the opponent can be ignored entirely: `Position::game_phase` reports a race
//! only when `x`'s rearmost checker is no further back than `o`'s rearmost one.
//! Two players can never share a pip, so in a race `o` holds no pip at or below
//! `x`'s highest occupied pip, and therefore can never block a move `x` wants to
//! make. `x`'s play is unobstructed, which is what lets the dynamic program below
//! look at `x`'s checkers alone.

use crate::position::Position;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Mean pips per roll: summed over all 36 outcomes, non-doubles give `a + b` and
/// doubles give `4a`, totalling 294; 294 / 36 = 8.1667.
const PIPS_PER_ROLL: f32 = 294.0 / 36.0;

/// Added to the value of any position that still has a checker outside the home
/// board, so every bear-off position sorts ahead of every non-bear-off one. This is
/// not a fudge factor: a checker can only be borne off once *all* checkers are home,
/// so a play that bears one off is always itself a bear-off position, and the two
/// groups never need to be compared on a shared numeric scale. Within the
/// non-bear-off group, ordering falls back to pip count.
const OUTSIDE_HOME_BASE: f32 = 100.0;

/// Checkers on pips 1..=6; index `i` holds the count on pip `i + 1`.
type BearOff = [u8; 6];

/// Pack a bear-off state into one integer. Each pip holds at most 15 checkers, so a
/// nibble each is enough.
fn state_key(state: &BearOff) -> u32 {
    state
        .iter()
        .rev()
        .fold(0u32, |acc, &c| (acc << 4) | c as u32)
}

/// Pack up to four dice (already in a canonical order) into one integer.
fn dice_key(dice: &[u8]) -> u32 {
    dice.iter().fold(1u32, |acc, &d| (acc << 4) | d as u32)
}

fn highest_occupied(state: &BearOff) -> usize {
    (1..=6).rev().find(|&p| state[p - 1] > 0).unwrap_or(0)
}

fn is_empty(state: &BearOff) -> bool {
    state.iter().all(|&c| c == 0)
}

/// Every position reachable by playing one die of value `d`, following the bear-off
/// rules that `Position::can_move` applies: bear off from pip `d` itself; bear off
/// from a lower pip only when nothing sits higher (so only from the highest occupied
/// pip); otherwise move a checker down from a pip above `d`.
fn successors(state: &BearOff, d: u8) -> Vec<BearOff> {
    let high = highest_occupied(state);
    if high == 0 {
        return Vec::new();
    }
    let die = d as usize;
    let mut out = Vec::with_capacity(6);

    if state[die - 1] > 0 {
        let mut next = *state;
        next[die - 1] -= 1;
        out.push(next);
    }
    // The die overshoots every checker, so the highest one comes off.
    if high < die {
        let mut next = *state;
        next[high - 1] -= 1;
        out.push(next);
    }
    for from in (die + 1)..=6 {
        if state[from - 1] > 0 {
            let mut next = *state;
            next[from - 1] -= 1;
            next[from - die - 1] += 1;
            out.push(next);
        }
    }
    out
}

/// Prefer the play that uses more dice (the rules demand maximum usage); among
/// those, the one that leaves the fewest expected rolls.
fn better(a: (u8, f32), b: (u8, f32)) -> (u8, f32) {
    match a.0.cmp(&b.0) {
        std::cmp::Ordering::Greater => a,
        std::cmp::Ordering::Less => b,
        std::cmp::Ordering::Equal => {
            if a.1 <= b.1 {
                a
            } else {
                b
            }
        }
    }
}

type Cache = Mutex<HashMap<(u32, u32), (u8, f32)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Best `(dice used, expected rolls afterwards)` from `state` given `dice` still to
/// play. Memoised, because a doubles roll would otherwise branch up to 6^4 ways.
fn play(state: &BearOff, dice: &[u8], cache: &Cache) -> (u8, f32) {
    if is_empty(state) {
        // Nothing left to bear off: the remaining dice are irrelevant, and counting
        // them as used keeps this branch from losing to a longer one.
        return (dice.len() as u8, 0.0);
    }
    if dice.is_empty() {
        return (0, expected_rolls_for_state(state, cache));
    }

    let memo = (state_key(state), dice_key(dice));
    if let Some(&hit) = cache.lock().unwrap().get(&memo) {
        return hit;
    }

    let mut best: Option<(u8, f32)> = None;
    let mut tried: Vec<u8> = Vec::with_capacity(2);
    for (i, &d) in dice.iter().enumerate() {
        if tried.contains(&d) {
            continue; // equal dice give identical play; only explore one of them
        }
        tried.push(d);
        for next in successors(state, d) {
            let mut rest: Vec<u8> = dice.to_vec();
            rest.remove(i);
            let (used, rolls) = play(&next, &rest, cache);
            let candidate = (used + 1, rolls);
            best = Some(match best {
                None => candidate,
                Some(current) => better(current, candidate),
            });
        }
    }
    // No die could be played at all, so the turn is forfeited.
    let result = best.unwrap_or_else(|| (0, expected_rolls_for_state(state, cache)));
    cache.lock().unwrap().insert(memo, result);
    result
}

/// Exact expected number of rolls to clear `state`, assuming best play.
///
/// Every die played either bears a checker off or moves one closer to home, so the
/// total pip count strictly decreases and the recursion is well founded. Memoised
/// lazily: only the states a real game actually visits get computed, rather than all
/// 54,264 of them up front.
fn expected_rolls_for_state(state: &BearOff, cache: &Cache) -> f32 {
    if is_empty(state) {
        return 0.0;
    }
    let memo = (state_key(state), 0);
    if let Some(&(_, hit)) = cache.lock().unwrap().get(&memo) {
        return hit;
    }

    let mut weighted = 0.0f32;
    for high in 1..=6u8 {
        for low in 1..=high {
            let dice: Vec<u8> = if high == low {
                vec![high; 4]
            } else {
                vec![high, low]
            };
            let (_, rolls) = play(state, &dice, cache);
            // Each of the 15 mixed rolls happens twice among the 36 outcomes.
            weighted += if high == low { rolls } else { 2.0 * rolls };
        }
    }
    let result = 1.0 + weighted / 36.0;
    cache.lock().unwrap().insert(memo, (0, result));
    result
}

/// How many rolls `x` still needs, as a sort key where lower is better.
///
/// Exact for a bear-off position (every checker on pips 1..=6). A position with a
/// checker still outside the home board gets [`OUTSIDE_HOME_BASE`] plus its pip
/// count in rolls, which keeps it behind every bear-off position while still
/// ordering sensibly against its own kind.
pub fn expected_rolls(position: &Position) -> f32 {
    let mut state: BearOff = [0; 6];
    let mut outside = false;
    // Index 25 is x's bar; 7..=24 are outside the home board.
    for pip in 1..=25usize {
        let checkers = position.pips[pip];
        if checkers <= 0 {
            continue;
        }
        if pip <= 6 {
            state[pip - 1] += checkers as u8;
        } else {
            outside = true;
        }
    }

    if outside {
        return OUTSIDE_HOME_BASE + pip_count(position) / PIPS_PER_ROLL;
    }
    if is_empty(&state) {
        return 0.0;
    }
    expected_rolls_for_state(&state, cache())
}

/// `x`'s pip count, counting the bar (index 25) at its full 25 pips.
fn pip_count(position: &Position) -> f32 {
    (1..=25usize)
        .map(|pip| {
            let checkers = position.pips[pip];
            if checkers > 0 {
                pip as f32 * checkers as f32
            } else {
                0.0
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::Dice;

    fn rolls(state: BearOff) -> f32 {
        expected_rolls_for_state(&state, cache())
    }

    /// A bear-off state plus a far-away opponent, so the position is a legal race.
    fn position_from(state: &BearOff) -> Position {
        let mut pips = [0i8; 26];
        for pip in 1..=6usize {
            pips[pip] = state[pip - 1] as i8;
        }
        // 15 opponent checkers parked on pip 24, far behind x's rearmost checker.
        pips[24] = -15;
        Position::try_from(pips).unwrap()
    }

    /// Every x-distribution reachable from `state` with this roll, per *our* rules.
    fn our_outcomes(state: &BearOff, a: u8, b: u8) -> Vec<BearOff> {
        let dice: Vec<u8> = if a == b { vec![a; 4] } else { vec![a, b] };
        let mut found = Vec::new();
        collect(state, &dice, &mut found);
        // The rules demand maximum dice usage, so keep only the longest plays.
        let most = found.iter().map(|&(used, _)| used).max().unwrap_or(0);
        let mut out: Vec<BearOff> = found
            .into_iter()
            .filter(|&(used, _)| used == most)
            .map(|(_, s)| s)
            .collect();
        out.sort();
        out.dedup();
        out
    }

    fn collect(state: &BearOff, dice: &[u8], out: &mut Vec<(u8, BearOff)>) {
        if is_empty(state) || dice.is_empty() {
            out.push((dice.len() as u8, *state));
            return;
        }
        let mut any = false;
        let mut tried: Vec<u8> = Vec::new();
        for (i, &d) in dice.iter().enumerate() {
            if tried.contains(&d) {
                continue;
            }
            tried.push(d);
            for next in successors(state, d) {
                any = true;
                let mut rest = dice.to_vec();
                rest.remove(i);
                let mut deeper = Vec::new();
                collect(&next, &rest, &mut deeper);
                for (left, s) in deeper {
                    out.push((left, s));
                }
            }
        }
        if !any {
            out.push((dice.len() as u8, *state));
        }
    }

    /// Same thing via the engine's own move generator, as an independent check.
    fn engine_outcomes(state: &BearOff, a: u8, b: u8) -> Vec<BearOff> {
        let position = position_from(state);
        let mut out: Vec<BearOff> = position
            .all_positions_after_moving(&Dice::new(a as usize, b as usize))
            .into_iter()
            // all_positions_after_moving hands back the opponent's view; switch back.
            .map(|p| {
                let p = p.sides_switched();
                let mut s: BearOff = [0; 6];
                for pip in 1..=6usize {
                    if p.pips[pip] > 0 {
                        s[pip - 1] = p.pips[pip] as u8;
                    }
                }
                s
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn bear_off_rules_match_the_engine_move_generator() {
        // Hand-rolled bear-off rules are easy to get subtly wrong, so check them
        // against Position::all_positions_after_moving over a spread of states.
        let states: Vec<BearOff> = vec![
            [2, 0, 0, 0, 0, 0],
            [3, 0, 0, 0, 0, 0],
            [1, 1, 1, 1, 1, 1],
            [0, 0, 0, 0, 0, 2],
            [0, 0, 1, 0, 0, 1],
            [3, 2, 2, 2, 3, 3],
            [0, 2, 0, 3, 0, 1],
            [5, 0, 0, 0, 0, 1],
        ];
        for state in states {
            for high in 1..=6u8 {
                for low in 1..=high {
                    assert_eq!(
                        our_outcomes(&state, high, low),
                        engine_outcomes(&state, high, low),
                        "state {state:?} roll {high}-{low}"
                    );
                }
            }
        }
    }

    #[test]
    fn single_checker_on_the_ace_point_takes_one_roll() {
        assert_eq!(rolls([1, 0, 0, 0, 0, 0]), 1.0);
    }

    #[test]
    fn single_checker_on_the_six_point() {
        // Not a certainty: it needs a 6, or two dice summing to at least 6 (move it
        // down, then bear it off as the only remaining checker). The rolls that fail
        // are 1-2, 1-3, 1-4, 2-3 (8 outcomes) and 1-1 (four aces reach only pip 2),
        // so 9 of 36 leave a checker that then comes off for sure next roll:
        // 1 + 9/36 = 1.25.
        assert_eq!(rolls([0, 0, 0, 0, 0, 1]), 1.25);
    }

    #[test]
    fn two_checkers_on_the_ace_point_take_one_roll() {
        assert_eq!(rolls([2, 0, 0, 0, 0, 0]), 1.0);
    }

    #[test]
    fn three_checkers_on_the_ace_point() {
        // Any double clears all three; anything else leaves one for a second roll.
        // 1 + (6*0 + 30*1)/36 = 1.8333...
        assert!((rolls([3, 0, 0, 0, 0, 0]) - (1.0 + 30.0 / 36.0)).abs() < 1e-6);
        // A fourth checker changes nothing: doubles still clear four.
        assert!((rolls([4, 0, 0, 0, 0, 0]) - (1.0 + 30.0 / 36.0)).abs() < 1e-6);
    }

    #[test]
    fn five_checkers_on_the_ace_point() {
        // Doubles leave one (E=1); other rolls leave three (E=1+30/36).
        let expected = 1.0 + (6.0 * 1.0 + 30.0 * (1.0 + 30.0 / 36.0)) / 36.0;
        assert!((rolls([5, 0, 0, 0, 0, 0]) - expected).abs() < 1e-6);
    }

    #[test]
    fn more_checkers_never_get_cheaper() {
        // Adding a checker cannot reduce the rolls needed.
        for pip in 1..=6usize {
            let mut fewer: BearOff = [1, 1, 1, 0, 0, 0];
            let mut more = fewer;
            more[pip - 1] += 1;
            assert!(
                rolls(more) >= rolls(fewer),
                "adding a checker on pip {pip} lowered the estimate"
            );
            fewer[pip - 1] += 3;
            assert!(rolls(fewer) >= rolls(more));
        }
    }

    #[test]
    fn checkers_further_back_never_get_cheaper() {
        // Moving a checker away from home cannot reduce the rolls needed.
        for pip in 1..=5usize {
            let mut nearer: BearOff = [0; 6];
            nearer[pip - 1] = 2;
            let mut further: BearOff = [0; 6];
            further[pip] = 2;
            assert!(
                rolls(further) >= rolls(nearer),
                "pip {} cheaper than pip {pip}",
                pip + 1
            );
        }
    }

    #[test]
    fn full_bear_off_is_plausible() {
        // 15 checkers spread over the home board. Bearing off 15 checkers takes at
        // least four rolls even with nothing but doubles, and a sane distribution
        // should not need more than a dozen.
        //
        // This is the worst case for the lazy build, since it reaches the most
        // states, so it doubles as a rough feel for the cost of a first race move
        // (a few hundred ms in release, a couple of seconds unoptimised). The
        // timing is printed rather than asserted: wall clock depends on the build
        // profile and on how loaded the machine is, which would make it flaky.
        let start = std::time::Instant::now();
        let value = rolls([3, 3, 3, 2, 2, 2]);
        let elapsed = start.elapsed();
        assert!(value > 4.0 && value < 12.0, "implausible: {value}");
        println!("15-checker bear-off: {value:.4} rolls, first call {elapsed:?}");
    }

    #[test]
    fn positions_outside_the_home_board_sort_behind_bear_offs() {
        let mut pips = [0i8; 26];
        pips[7] = 1; // a single checker one pip outside the home board
        pips[1] = 14;
        pips[24] = -15;
        let outside = Position::try_from(pips).unwrap();

        let mut pips = [0i8; 26];
        pips[1] = 15; // everything home, and far more pips left to grind out
        pips[24] = -15;
        let home = Position::try_from(pips).unwrap();

        assert!(expected_rolls(&outside) > expected_rolls(&home));
        // ... and fewer pips still sorts ahead within the outside-home group.
        let mut pips = [0i8; 26];
        pips[20] = 1;
        pips[1] = 14;
        pips[24] = -15;
        let far_outside = Position::try_from(pips).unwrap();
        assert!(expected_rolls(&far_outside) > expected_rolls(&outside));
    }
}
