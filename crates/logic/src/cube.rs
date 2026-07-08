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

#[cfg_attr(feature = "web", derive(Serialize, ToSchema))]
#[cfg_attr(feature = "web", serde(rename_all = "camelCase"))]
/// Money game cube decisions based on Janowski's cube formulae.
///
/// See <https://bkgm.com/articles/Janowski/cubeformulae.pdf>.
///
/// The decisions assume a money game. Cube ownership is taken into account via
/// [`CubePosition`]: when the opponent owns the cube, player `x` may not double
/// and both `double` and `accept` are `false`. Match play is a future extension.
pub struct CubeInfo {
    /// `true` if the player `x` should double, `false` if no double yet or too good.
    double: bool,
    /// `true` if the opponent should take the cube, `false` if they should reject.
    accept: bool,
    /// Cubeless money game equity of the position, from player `x`'s perspective.
    cubeless_equity: f32,
    /// Cubeful equity if player `x` does not double, from `x`'s perspective.
    /// This depends on the current cube position.
    equity_no_double: f32,
    /// Cubeful equity if player `x` doubles and the opponent takes, from `x`'s perspective.
    /// The stake is already doubled, so this is comparable to the other equities.
    equity_double_take: f32,
}

impl CubeInfo {
    /// Computes the cube decision for the given probabilities and cube position.
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

        // `x` can only offer a double when holding a centered cube or owning it.
        let can_double = matches!(cube_position, CubePosition::Centered | CubePosition::Owned);

        // The opponent takes when doing so is better for them than dropping.
        // Dropping costs them the current stake (`x`'s equity would be +1.0).
        let accept = can_double && equity_double_take < 1.0;
        // `x` doubles when doubling – after the opponent's best response of
        // take or drop – beats not doubling. Positions that are "too good" to
        // double yield `equity_no_double > 1.0` and thus `false`.
        let double = can_double && equity_double_take.min(1.0) > equity_no_double;

        Self {
            double,
            accept,
            cubeless_equity: value.equity(),
            equity_no_double,
            equity_double_take,
        }
    }
}

impl From<&Probabilities> for CubeInfo {
    /// Cube decision for a centered cube, i.e. an initial double decision.
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
    use super::{CubeInfo, CubePosition};
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
}
