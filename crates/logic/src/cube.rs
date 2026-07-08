use crate::match_equity::{
    MAX_AWAY, match_equity_after_loss, match_equity_after_win, position_equity,
};
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

/// Largest doubling cube value the match cube math will consider. Real cubes are
/// far below this; the bound only keeps the take-point recursion from overflowing.
pub const MAX_CUBE_VALUE: u32 = 1 << 20;

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

impl MatchState {
    /// Builds the scoring context from away scores: both `0` is a money game,
    /// otherwise it is match play. Validates the away scores and, if `crawford`
    /// is set, that exactly one player is one point away. This is the single
    /// place that maps away scores to a [`MatchState`].
    pub fn from_away(x_away: u32, o_away: u32, crawford: bool) -> Result<Self, &'static str> {
        match (x_away, o_away) {
            (0, 0) => Ok(MatchState::Money),
            (0, _) | (_, 0) => Err("for match play both x_away and o_away must be at least 1"),
            (x_away, o_away) if x_away <= MAX_AWAY && o_away <= MAX_AWAY => {
                if crawford && (x_away == 1) == (o_away == 1) {
                    return Err(
                        "the Crawford game requires exactly one player to be one point away",
                    );
                }
                Ok(MatchState::Match {
                    x_away,
                    o_away,
                    crawford,
                })
            }
            _ => Err("away scores larger than the match equity table are not supported"),
        }
    }
}

/// Whether player `x` is allowed to (re)double given the cube position.
fn can_double(position: CubePosition) -> bool {
    matches!(position, CubePosition::Centered | CubePosition::Owned)
}

#[cfg_attr(feature = "web", derive(Serialize, ToSchema))]
#[cfg_attr(feature = "web", serde(rename_all = "camelCase"))]
/// Cube decisions for money game (Janowski's cube formulae) or match play (a
/// live-cube model with recursive take points on top of a match equity table).
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

    /// Match play cube decision using the live-cube model (recursive live take
    /// points on top of the match equity table). `x_away`/`o_away` are how many
    /// points each player still needs; `crawford` marks the Crawford game.
    pub fn for_match(
        value: &Probabilities,
        cube: CubeState,
        x_away: u32,
        o_away: u32,
        crawford: bool,
    ) -> Self {
        let cubeless_equity = value.equity();
        // Clamp inputs so the take-point recursion always terminates and cannot
        // overflow, whatever a caller passes (`match_equity` clamps aways too).
        let stake = cube.value.clamp(1, MAX_CUBE_VALUE);
        let x_away = x_away.min(MAX_AWAY);
        let o_away = o_away.min(MAX_AWAY);

        // During the Crawford game the cube may not be used.
        if crawford {
            let e_no_double = position_equity(value, x_away, o_away, stake);
            let e_double_take = position_equity(value, x_away, o_away, 2 * stake);
            return Self::no_cube(cubeless_equity, e_no_double, e_double_take);
        }

        // Post-Crawford: exactly one player is one point away.
        if x_away == 1 || o_away == 1 {
            let (double, accept) = post_crawford_decision(x_away, can_double(cube.position));
            return Self {
                double,
                accept,
                cubeless_equity,
                equity_no_double: position_equity(value, x_away, o_away, stake),
                equity_double_take: position_equity(value, x_away, o_away, 2 * stake),
            };
        }

        // Pre-Crawford: live-cube cubeful equities, differentiated by cube owner.
        // A centered cube uses the cubeless play-on value as its "no double"
        // baseline (the conventional static reference for an initial double);
        // owning the cube adds the value of being able to cash, which correctly
        // raises the bar for a redouble.
        let equity_no_double = match cube.position {
            CubePosition::Centered => position_equity(value, x_away, o_away, stake),
            CubePosition::Owned => equity_owner_can_cash(value, x_away, o_away, stake),
            CubePosition::OpponentOwned => equity_opponent_can_cash(value, x_away, o_away, stake),
        };
        // If `x` doubles and it is taken, the opponent owns the doubled cube.
        let equity_double_take = equity_opponent_can_cash(value, x_away, o_away, 2 * stake);
        // If the opponent passes, `x` cashes the current stake.
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

/// Match-winning probability for `x` given `x` wins the game, playing at cube
/// value `v` (gammon-weighted). This is the `p → 1` endpoint of the equity line.
fn win_equity(value: &Probabilities, a: u32, b: u32, v: u32) -> f32 {
    let p = value.win();
    if p <= 0.0 {
        return 0.0;
    }
    (value.win_normal * match_equity_after_win(a, b, v)
        + value.win_gammon * match_equity_after_win(a, b, 2 * v)
        + value.win_bg * match_equity_after_win(a, b, 3 * v))
        / p
}

/// Match-winning probability for `x` given `x` loses the game, playing at cube
/// value `v` (gammon-weighted). This is the `p → 0` endpoint of the equity line.
fn lose_equity(value: &Probabilities, a: u32, b: u32, v: u32) -> f32 {
    let q = 1.0 - value.win();
    if q <= 0.0 {
        return 1.0;
    }
    (value.lose_normal * match_equity_after_loss(a, b, v)
        + value.lose_gammon * match_equity_after_loss(a, b, 2 * v)
        + value.lose_bg * match_equity_after_loss(a, b, 3 * v))
        / q
}

/// Live-cube take point (as a win probability) for the player who needs
/// `taker_away` points and is being doubled to cube value `cube` against an
/// opponent who needs `opp_away`. Ignoring gammons, the dead take point comes
/// straight from the match equity table; the recube option that the taker gains
/// by owning the cube lowers it via `TP_live(n) = TP_dead(n)·(1 − TP_live(2n))`.
/// The recursion stops once a single win at the current cube wins the match, so
/// it self-caps at the finite match and always terminates.
fn live_take_point(taker_away: u32, opp_away: u32, cube: u32) -> f32 {
    let declined = cube / 2;
    let pass = match_equity_after_loss(taker_away, opp_away, declined);
    let win = match_equity_after_win(taker_away, opp_away, cube);
    let lose = match_equity_after_loss(taker_away, opp_away, cube);
    let denom = win - lose;
    let dead = if denom <= 1e-9 {
        0.0
    } else {
        ((pass - lose) / denom).clamp(0.0, 1.0)
    };
    if cube >= taker_away {
        // A single win at this cube already wins the match: no recube value.
        dead
    } else {
        let recube = live_take_point(opp_away, taker_away, 2 * cube);
        dead * (1.0 - recube)
    }
}

/// Cubeful match equity for `x` when the cube (value `v`) is owned by `x`, so
/// only `x` can (re)double and cash. Piecewise-linear between losing the game and
/// `x`'s cash point (where the opponent reaches its live take point).
fn equity_owner_can_cash(value: &Probabilities, a: u32, b: u32, v: u32) -> f32 {
    let p = value.win();
    if p >= 1.0 {
        return win_equity(value, a, b, v);
    }
    let cash = match_equity_after_win(a, b, v);
    let cash_point = (1.0 - live_take_point(b, a, 2 * v)).clamp(0.0, 1.0);
    if p >= cash_point || cash_point <= 0.0 {
        return cash;
    }
    let lose = lose_equity(value, a, b, v);
    lose + (cash - lose) * (p / cash_point)
}

/// Cubeful match equity for `x` when the cube (value `v`) is owned by the
/// opponent, so only the opponent can (re)double and cash. Piecewise-linear
/// between the opponent cashing (at `x`'s live take point) and `x` winning.
fn equity_opponent_can_cash(value: &Probabilities, a: u32, b: u32, v: u32) -> f32 {
    let p = value.win();
    if p <= 0.0 {
        return lose_equity(value, a, b, v);
    }
    let anti_cash = match_equity_after_loss(a, b, v);
    let take_point = live_take_point(a, b, 2 * v).clamp(0.0, 1.0);
    if p <= take_point {
        return anti_cash;
    }
    let win = win_equity(value, a, b, v);
    anti_cash + (win - anti_cash) * (p - take_point) / (1.0 - take_point)
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
        // `x` is the trailer; double immediately (if it can). The leader takes
        // unless a free drop applies, i.e. unless the trailer needs an even
        // number of points. `accept` is only meaningful when `x` can double.
        (can_double, can_double && x_away % 2 == 1)
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

    #[test]
    fn match_cube_ownership_is_differentiated() {
        // Unlike the old static method, owning the cube is worth more than a
        // centered cube, which is worth more than the opponent owning it.
        let probs = no_gammons(0.60);
        let no_double = |position| {
            CubeInfo::for_match(&probs, CubeState { position, value: 1 }, 5, 5, false)
                .equity_no_double()
        };
        assert!(no_double(CubePosition::Owned) > no_double(CubePosition::Centered));
        assert!(no_double(CubePosition::Centered) > no_double(CubePosition::OpponentOwned));
    }

    #[test]
    fn match_state_from_away_validates() {
        use super::MatchState;
        assert_eq!(MatchState::from_away(0, 0, false), Ok(MatchState::Money));
        assert!(MatchState::from_away(0, 5, false).is_err());
        assert!(MatchState::from_away(5, 0, false).is_err());
        assert!(MatchState::from_away(20, 20, false).is_err());
        assert_eq!(
            MatchState::from_away(2, 2, false),
            Ok(MatchState::Match {
                x_away: 2,
                o_away: 2,
                crawford: false
            })
        );
        // Crawford needs exactly one player one point away.
        assert!(MatchState::from_away(3, 1, true).is_ok());
        assert!(MatchState::from_away(5, 5, true).is_err());
        assert!(MatchState::from_away(1, 1, true).is_err());
    }

    #[test]
    fn match_cube_value_zero_does_not_crash() {
        // A degenerate cube value must be clamped, not send the take-point
        // recursion into an infinite loop.
        let cube = CubeState {
            position: CubePosition::Centered,
            value: 0,
        };
        let info = CubeInfo::for_match(&no_gammons(0.6), cube, 5, 5, false);
        assert!(info.equity_no_double().is_finite());
    }

    #[test]
    fn post_crawford_leader_cube_cannot_be_taken() {
        // Trailer with an odd away would normally be taken, but if the opponent
        // owns the cube `x` cannot double, so `accept` must be false too.
        let cube = CubeState {
            position: CubePosition::OpponentOwned,
            value: 1,
        };
        let info = CubeInfo::for_match(&no_gammons(0.5), cube, 3, 1, false);
        assert!(!info.double());
        assert!(!info.accept());
    }

    #[test]
    fn deep_match_take_point_has_recube_vig() {
        // Deep in a long match the cube is fully live: the recube option pulls
        // the take point down from the dead-cube 25% to about the money-game
        // live value of 20%.
        let live = super::live_take_point(15, 15, 2);
        assert!((live - 0.20).abs() < 0.03, "live take point was {live}");
        assert!(
            live < 0.25,
            "recube vig should lower the take point below 25%"
        );
    }
}
