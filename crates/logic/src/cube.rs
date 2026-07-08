use crate::match_equity::{match_equity_after_loss, match_equity_after_win};
use engine::probabilities::Probabilities;
#[cfg(feature = "web")]
use serde::Serialize;
#[cfg(feature = "web")]
use utoipa::ToSchema;

/// Cube efficiency (also called "cube-life index") as suggested by Janowski.
/// `0.0` would model a completely dead cube, `1.0` a fully live (continuous) cube.
/// `2/3` is Janowski's recommended value for a typical money game.
const CUBE_EFFICIENCY: f32 = 2.0 / 3.0;

/// Who currently owns the doubling cube, seen from player `x`'s perspective.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CubePosition {
    /// The cube is in the middle; either player may double. This is the state
    /// before anyone has doubled and the default assumed for an initial double.
    #[default]
    Centered,
    /// Player `x` owns the cube and is the only one who may (re)double.
    Owned,
    /// The opponent owns the cube; player `x` may not double.
    OpponentOwned,
}

/// The current state of the doubling cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CubeState {
    /// Who owns the cube.
    pub position: CubePosition,
    /// Current cube value (1, 2, 4, …). Only relevant in match play; a money
    /// game is linear, so the value does not change the decision there.
    pub value: u32,
}

impl Default for CubeState {
    fn default() -> Self {
        Self {
            position: CubePosition::Centered,
            value: 1,
        }
    }
}

/// The scoring context for a cube decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchState {
    /// Unlimited money game; decided with Janowski's cube formulae.
    Money,
    /// Match play, using a match equity table. `x_away`/`o_away` are how many
    /// points each player still needs; `crawford` marks the Crawford game.
    Match {
        x_away: u32,
        o_away: u32,
        crawford: bool,
    },
}

/// Whether player `x` is allowed to (re)double given the cube position.
fn can_double(position: CubePosition) -> bool {
    matches!(position, CubePosition::Centered | CubePosition::Owned)
}

#[cfg_attr(feature = "web", derive(Serialize, ToSchema))]
#[cfg_attr(feature = "web", serde(rename_all = "camelCase"))]
/// Cube decisions for money game (Janowski's cube formulae) or match play (a
/// static take-point method on top of a match equity table).
///
/// See <https://bkgm.com/articles/Janowski/cubeformulae.pdf> for the money game.
///
/// Cube ownership is taken into account via [`CubePosition`]: when the opponent
/// owns the cube, player `x` may not double and both `double` and `accept` are
/// `false`. In match play the Crawford and post-Crawford rules are handled too.
///
/// The `equity_*` fields are cubeful equities in points for a money game, but
/// match-winning probabilities (0..1) for match play; `cubeless_equity` is
/// always the money cubeless equity in points.
pub struct CubeInfo {
    /// `true` if the player `x` should double, `false` if no double yet or too good.
    double: bool,
    /// `true` if the opponent should take the cube, `false` if they should reject.
    accept: bool,
    /// Cubeless money game equity of the position, from player `x`'s perspective.
    cubeless_equity: f32,
    /// Equity if player `x` does not double, from `x`'s perspective.
    /// Depends on the current cube position (and, in match play, the score).
    equity_no_double: f32,
    /// Equity if player `x` doubles and the opponent takes, from `x`'s perspective.
    /// The stake is already doubled, so this is comparable to the other equities.
    equity_double_take: f32,
}

impl CubeInfo {
    /// Cube decision for the given probabilities, cube state and scoring context.
    pub fn for_state(value: &Probabilities, cube: CubeState, match_state: MatchState) -> Self {
        match match_state {
            MatchState::Money => Self::new(value, cube.position),
            MatchState::Match {
                x_away,
                o_away,
                crawford,
            } => Self::for_match(value, cube, x_away, o_away, crawford),
        }
    }

    /// Money game cube decision for the given cube position (Janowski).
    pub fn new(value: &Probabilities, cube_position: CubePosition) -> Self {
        let x = CUBE_EFFICIENCY;

        // Probability of winning and losing (cubeless).
        let p = value.win();
        let q = 1.0 - p;

        // Average points won given a win (`w`) and lost given a loss (`l`).
        // Both are positive and lie in `[1, 3]`. Guard against division by zero
        // for certain wins or losses, in which case that side never happens.
        let w = if p > 0.0 {
            (value.win_normal + 2.0 * value.win_gammon + 3.0 * value.win_bg) / p
        } else {
            0.0
        };
        let l = if q > 0.0 {
            (value.lose_normal + 2.0 * value.lose_gammon + 3.0 * value.lose_bg) / q
        } else {
            0.0
        };

        // Janowski's cubeful equities from `x`'s perspective for a cube of value 1,
        // one for each possible cube position.
        let common = p * (w + l + 0.5 * x) - l;
        // Centered cube: neither player has doubled yet.
        let equity_centered = (4.0 / (4.0 - x)) * (common - 0.25 * x);
        // Player `x` owns the cube.
        let equity_owned = common;
        // The opponent owns the cube (also the state after `x` doubles and it is taken).
        let equity_opponent_owned = common - 0.5 * x;

        // Equity if `x` does not double, depending on who owns the cube.
        let equity_no_double = match cube_position {
            CubePosition::Centered => equity_centered,
            CubePosition::Owned => equity_owned,
            CubePosition::OpponentOwned => equity_opponent_owned,
        };
        // After a double and take the opponent owns the cube and the stake is doubled.
        let equity_double_take = 2.0 * equity_opponent_owned;
        // If the opponent passes, `x` cashes the current stake, worth +1.0 per point.
        let equity_pass = 1.0;

        Self::decide(
            value.equity(),
            equity_no_double,
            equity_pass,
            equity_double_take,
            can_double(cube_position),
        )
    }

    /// Match play cube decision using a static take-point method on top of the
    /// match equity table. `x_away`/`o_away` are how many points each player
    /// still needs; `crawford` marks the Crawford game.
    pub fn for_match(
        value: &Probabilities,
        cube: CubeState,
        x_away: u32,
        o_away: u32,
        crawford: bool,
    ) -> Self {
        let cubeless_equity = value.equity();
        let stake = cube.value;
        // Match-winning probability of just playing on at the current / doubled cube.
        let equity_no_double = expected_match_equity(value, x_away, o_away, stake);
        let equity_double_take = expected_match_equity(value, x_away, o_away, 2 * stake);

        // During the Crawford game the cube may not be used.
        if crawford {
            return Self::no_cube(cubeless_equity, equity_no_double, equity_double_take);
        }

        // Post-Crawford: exactly one player is one point away.
        if x_away == 1 || o_away == 1 {
            let (double, accept) = post_crawford_decision(x_away, can_double(cube.position));
            return Self {
                double,
                accept,
                cubeless_equity,
                equity_no_double,
                equity_double_take,
            };
        }

        // Pre-Crawford static take-point method. If the opponent passes, `x`
        // cashes the current stake (`stake` points before the double).
        let equity_pass = match_equity_after_win(x_away, o_away, stake);
        Self::decide(
            cubeless_equity,
            equity_no_double,
            equity_pass,
            equity_double_take,
            can_double(cube.position),
        )
    }

    /// Derives `double`/`accept` from the equities of the three cube actions:
    /// not doubling (`equity_no_double`), the opponent passing (`equity_pass`)
    /// and the opponent taking (`equity_double_take`). Works for money game
    /// (points) and match play (match-winning probabilities) alike.
    fn decide(
        cubeless_equity: f32,
        equity_no_double: f32,
        equity_pass: f32,
        equity_double_take: f32,
        can_double: bool,
    ) -> Self {
        // The opponent picks the response that is worst for `x`.
        let equity_double = equity_pass.min(equity_double_take);
        // The opponent takes when taking is better for them than passing.
        let accept = can_double && equity_double_take < equity_pass;
        // `x` doubles when doubling beats not doubling. "Too good" positions,
        // where playing on is worth more than cashing, fall out automatically.
        let double = can_double && equity_double > equity_no_double;
        Self {
            double,
            accept,
            cubeless_equity,
            equity_no_double,
            equity_double_take,
        }
    }

    /// A decision where the cube cannot be used (e.g. the Crawford game).
    fn no_cube(cubeless_equity: f32, equity_no_double: f32, equity_double_take: f32) -> Self {
        Self {
            double: false,
            accept: false,
            cubeless_equity,
            equity_no_double,
            equity_double_take,
        }
    }
}

/// Expected match-winning probability for `x` of playing the game out at the
/// given `stake` (the number of points a plain win is worth), summed over the
/// cubeless win/gammon/backgammon distribution.
fn expected_match_equity(value: &Probabilities, x_away: u32, o_away: u32, stake: u32) -> f32 {
    value.win_normal * match_equity_after_win(x_away, o_away, stake)
        + value.win_gammon * match_equity_after_win(x_away, o_away, 2 * stake)
        + value.win_bg * match_equity_after_win(x_away, o_away, 3 * stake)
        + value.lose_normal * match_equity_after_loss(x_away, o_away, stake)
        + value.lose_gammon * match_equity_after_loss(x_away, o_away, 2 * stake)
        + value.lose_bg * match_equity_after_loss(x_away, o_away, 3 * stake)
}

/// Cube decision in a post-Crawford game, where exactly one player is one point
/// away. The trailer should double immediately; the leader never doubles. The
/// leader's take follows the free-drop rule: drop when the trailer needs an even
/// number of points, otherwise take.
fn post_crawford_decision(x_away: u32, can_double: bool) -> (bool, bool) {
    if x_away == 1 {
        // `x` is the leader (needs one point); doubling can never help.
        (false, false)
    } else {
        // `x` is the trailer; double immediately. The leader takes unless a free
        // drop applies, i.e. unless the trailer needs an even number of points.
        (can_double, x_away % 2 == 1)
    }
}

impl From<&Probabilities> for CubeInfo {
    /// Cube decision for a centered cube in a money game, i.e. an initial double.
    fn from(value: &Probabilities) -> Self {
        Self::new(value, CubePosition::Centered)
    }
}

impl CubeInfo {
    pub fn double(&self) -> bool {
        self.double
    }
    pub fn accept(&self) -> bool {
        self.accept
    }
    pub fn cubeless_equity(&self) -> f32 {
        self.cubeless_equity
    }
    pub fn equity_no_double(&self) -> f32 {
        self.equity_no_double
    }
    pub fn equity_double_take(&self) -> f32 {
        self.equity_double_take
    }
}

#[cfg(test)]
mod tests {
    use super::{CubeInfo, CubePosition, CubeState, MatchState};
    use engine::probabilities::Probabilities;

    /// Helper for a position without gammons or backgammons and a given win probability.
    fn no_gammons(win: f32) -> Probabilities {
        Probabilities {
            win_normal: win,
            win_gammon: 0.0,
            win_bg: 0.0,
            lose_normal: 1.0 - win,
            lose_gammon: 0.0,
            lose_bg: 0.0,
        }
    }

    #[test]
    fn symmetric_position_is_neutral() {
        // Given a completely symmetric position (50% wins, no gammons)
        let cube = CubeInfo::from(&no_gammons(0.5));
        // Then equities are zero, there is no double and a take is correct.
        assert!(cube.cubeless_equity().abs() < 1e-6);
        assert!(cube.equity_no_double().abs() < 1e-6);
        assert!(!cube.double());
        assert!(cube.accept());
    }

    #[test]
    fn double_take_window() {
        // Given a clear advantage that is still takeable (around 70% wins)
        let cube = CubeInfo::from(&no_gammons(0.70));
        // Then the player should double and the opponent should take.
        assert!(cube.double());
        assert!(cube.accept());
    }

    #[test]
    fn too_strong_to_be_taken() {
        // Given a huge racing lead (85% wins, no gammons)
        let cube = CubeInfo::from(&no_gammons(0.85));
        // Then the player should double but the opponent must drop.
        assert!(cube.double());
        assert!(!cube.accept());
    }

    #[test]
    fn no_double_when_barely_ahead() {
        // Given only a small advantage (55% wins)
        let cube = CubeInfo::from(&no_gammons(0.55));
        // Then it is too early to double, but a take would be trivial.
        assert!(!cube.double());
        assert!(cube.accept());
    }

    #[test]
    fn too_good_to_double() {
        // Given a position that wins a lot of gammons and backgammons,
        // playing on is worth more than a single point from doubling out.
        let probs = Probabilities {
            win_normal: 0.15,
            win_gammon: 0.55,
            win_bg: 0.2,
            lose_normal: 0.1,
            lose_gammon: 0.0,
            lose_bg: 0.0,
        };
        let cube = CubeInfo::from(&probs);
        // Then the player is too good to double (playing on beats cashing).
        assert!(cube.equity_no_double() > 1.0);
        assert!(!cube.double());
    }

    #[test]
    fn from_probabilities_defaults_to_centered() {
        // `From<&Probabilities>` should behave like an initial double decision.
        let probs = no_gammons(0.70);
        let from = CubeInfo::from(&probs);
        let centered = CubeInfo::new(&probs, CubePosition::Centered);
        assert_eq!(from.double(), centered.double());
        assert_eq!(from.accept(), centered.accept());
        assert_eq!(from.equity_no_double(), centered.equity_no_double());
    }

    #[test]
    fn redouble_needs_more_than_initial_double() {
        // Owning the cube is worth something, so a position that is a clear
        // initial double may not yet be worth redoubling: the "no double"
        // baseline is higher when you already own the cube.
        let probs = no_gammons(0.68);
        let centered = CubeInfo::new(&probs, CubePosition::Centered);
        let owned = CubeInfo::new(&probs, CubePosition::Owned);
        // Same take offer either way (opponent ends up owning the cube).
        assert_eq!(centered.equity_double_take(), owned.equity_double_take());
        // But owning the cube raises the "no double" equity above the centered one.
        assert!(owned.equity_no_double() > centered.equity_no_double());
        // At this win rate it is an initial double but not yet a redouble.
        assert!(centered.double());
        assert!(!owned.double());
    }

    #[test]
    fn cannot_double_when_opponent_owns_cube() {
        // Even with a commanding lead, `x` may not double if the opponent owns the cube.
        let cube = CubeInfo::new(&no_gammons(0.80), CubePosition::OpponentOwned);
        assert!(!cube.double());
        assert!(!cube.accept());
        // The no-double equity reflects the opponent owning the cube.
        let expected = CubeInfo::new(&no_gammons(0.80), CubePosition::Centered);
        assert!(cube.equity_no_double() < expected.equity_no_double());
    }

    // --- Match play ---

    /// A centered cube of value 1 at the given away score.
    fn at_match(win: f32, x_away: u32, o_away: u32, crawford: bool) -> CubeInfo {
        CubeInfo::for_match(
            &no_gammons(win),
            CubeState::default(),
            x_away,
            o_away,
            crawford,
        )
    }

    #[test]
    fn for_state_money_matches_new() {
        // The money variant of `for_state` must equal the plain money constructor.
        let probs = no_gammons(0.70);
        let cube = CubeState {
            position: CubePosition::Owned,
            value: 4,
        };
        let via_state = CubeInfo::for_state(&probs, cube, MatchState::Money);
        let direct = CubeInfo::new(&probs, CubePosition::Owned);
        assert_eq!(via_state.double(), direct.double());
        assert_eq!(via_state.accept(), direct.accept());
        assert_eq!(via_state.equity_no_double(), direct.equity_no_double());
        assert_eq!(via_state.equity_double_take(), direct.equity_double_take());
    }

    #[test]
    fn no_cube_during_crawford_game() {
        // In the Crawford game the cube may not be used, whatever the position.
        let cube = at_match(0.80, 3, 1, true);
        assert!(!cube.double());
        assert!(!cube.accept());
    }

    #[test]
    fn post_crawford_trailer_doubles_leader_does_not() {
        // Trailer (needs more than one point) doubles immediately.
        let trailer = at_match(0.50, 3, 1, false);
        assert!(trailer.double());
        // Leader (needs one point) never doubles.
        let leader = at_match(0.50, 1, 3, false);
        assert!(!leader.double());
    }

    #[test]
    fn post_crawford_free_drop_on_even_away() {
        // Trailer needs an odd number of points: the leader takes.
        assert!(at_match(0.50, 3, 1, false).accept());
        // Trailer needs an even number of points: the leader has a free drop.
        assert!(!at_match(0.50, 4, 1, false).accept());
    }

    #[test]
    fn two_away_two_away_take_point_is_about_thirty_percent() {
        // At 2-away/2-away a doubled game is played for the match, so the taker's
        // take point is about 30%: the opponent takes when x wins < 70%.
        assert!(at_match(0.60, 2, 2, false).accept());
        assert!(!at_match(0.75, 2, 2, false).accept());
    }

    #[test]
    fn two_away_two_away_doubles_around_fifty_percent() {
        // At 2-away/2-away you should double as soon as you are a real favorite.
        assert!(!at_match(0.40, 2, 2, false).double());
        assert!(at_match(0.60, 2, 2, false).double());
    }

    #[test]
    fn match_and_money_can_disagree() {
        // At 55% with no gammons a money game is not yet a double, but at
        // 2-away/2-away the same position clearly is.
        let probs = no_gammons(0.55);
        assert!(!CubeInfo::from(&probs).double());
        assert!(at_match(0.55, 2, 2, false).double());
    }

    #[test]
    fn match_opponent_owning_cube_forbids_double() {
        // `x` may not double when the opponent owns the cube, even in a match.
        let cube = CubeState {
            position: CubePosition::OpponentOwned,
            value: 2,
        };
        let info = CubeInfo::for_match(&no_gammons(0.80), cube, 5, 5, false);
        assert!(!info.double());
        assert!(!info.accept());
    }
}
