//! Match equity table (MET): the probability of winning a match at a given score.
//!
//! The values are Kit Woolsey's published match equity table, from
//! *How to Play Tournament Backgammon* (1993). The underlying gammon-rate and
//! cube-leverage analysis was made possible by a large database provided by
//! Hal Heinrich. See <https://bkgm.com/articles/Woolsey/TheMatchEquityTable/>.
//!
//! `WOOLSEY[i - 1][j - 1]` is the winning chance, in percent, of the player who
//! needs `i` more points against an opponent who needs `j` more points. The
//! entries where either player is one point away already assume the Crawford
//! game (no doubling). The table is complementary: `M[i][j] + M[j][i] == 100`.

use engine::probabilities::Probabilities;

/// Highest away-score the table covers. Larger scores are clamped to this.
pub const MAX_AWAY: u32 = 15;

/// Kit Woolsey's match equity table, as whole-percent winning chances.
#[rustfmt::skip]
const WOOLSEY: [[u8; MAX_AWAY as usize]; MAX_AWAY as usize] = [
    [50, 70, 75, 83, 85, 90, 91, 94, 95, 97, 97, 98, 98, 99, 99],
    [30, 50, 60, 68, 75, 81, 85, 88, 91, 93, 94, 95, 96, 97, 98],
    [25, 40, 50, 59, 66, 71, 76, 80, 84, 87, 90, 92, 94, 95, 96],
    [17, 32, 41, 50, 58, 64, 70, 75, 79, 83, 86, 88, 90, 92, 93],
    [15, 25, 34, 42, 50, 57, 63, 68, 73, 77, 81, 84, 87, 89, 90],
    [10, 19, 29, 36, 43, 50, 56, 62, 67, 72, 76, 79, 82, 85, 87],
    [ 9, 15, 24, 30, 37, 44, 50, 56, 61, 66, 70, 74, 78, 81, 84],
    [ 6, 12, 20, 25, 32, 38, 44, 50, 55, 60, 65, 69, 73, 77, 80],
    [ 5,  9, 16, 21, 27, 33, 39, 45, 50, 55, 60, 64, 68, 72, 76],
    [ 3,  7, 13, 17, 23, 28, 34, 40, 45, 50, 55, 60, 64, 68, 71],
    [ 3,  6, 10, 14, 19, 24, 30, 35, 40, 45, 50, 55, 59, 63, 67],
    [ 2,  5,  8, 12, 16, 21, 26, 31, 36, 40, 45, 50, 54, 58, 62],
    [ 2,  4,  6, 10, 13, 18, 22, 27, 32, 36, 41, 46, 50, 54, 58],
    [ 1,  3,  5,  8, 11, 15, 19, 23, 28, 32, 37, 42, 46, 50, 54],
    [ 1,  2,  4,  7, 10, 13, 16, 20, 24, 29, 33, 38, 42, 46, 50],
];

/// Probability that player `x`, who needs `x_away` points, wins the match against
/// an opponent who needs `o_away` points.
///
/// An away score of `0` means that player has already won the match. Away scores
/// larger than [`MAX_AWAY`] are clamped to `MAX_AWAY`.
pub fn match_equity(x_away: u32, o_away: u32) -> f32 {
    if x_away == 0 {
        return 1.0;
    }
    if o_away == 0 {
        return 0.0;
    }
    let i = x_away.min(MAX_AWAY) as usize - 1;
    let j = o_away.min(MAX_AWAY) as usize - 1;
    WOOLSEY[i][j] as f32 / 100.0
}

/// Match equity for player `x` after winning `points` at the score
/// (`x_away`, `o_away`). Winning at least `x_away` points wins the match.
pub fn match_equity_after_win(x_away: u32, o_away: u32, points: u32) -> f32 {
    if points >= x_away {
        1.0
    } else {
        match_equity(x_away - points, o_away)
    }
}

/// Match equity for player `x` after losing `points` at the score
/// (`x_away`, `o_away`). Losing at least `o_away` points loses the match.
pub fn match_equity_after_loss(x_away: u32, o_away: u32, points: u32) -> f32 {
    if points >= o_away {
        0.0
    } else {
        match_equity(x_away, o_away - points)
    }
}

/// Expected match-winning probability for player `x` of playing the game out to
/// conclusion at the given `cube_value`, summed over the cubeless
/// win/gammon/backgammon distribution. A win of `k` points moves the score by
/// `k`; points that clinch the match are capped. This is a cubeless match
/// evaluation, used both to rank moves and as a building block for the cube.
///
/// It is symmetric under switching sides (`equity(switch(p)) == 1 - equity(p)`),
/// which is required for correct move ranking from either player's perspective.
pub fn position_equity(probs: &Probabilities, x_away: u32, o_away: u32, cube_value: u32) -> f32 {
    let c = cube_value;
    probs.win_normal * match_equity_after_win(x_away, o_away, c)
        + probs.win_gammon * match_equity_after_win(x_away, o_away, 2 * c)
        + probs.win_bg * match_equity_after_win(x_away, o_away, 3 * c)
        + probs.lose_normal * match_equity_after_loss(x_away, o_away, c)
        + probs.lose_gammon * match_equity_after_loss(x_away, o_away, 2 * c)
        + probs.lose_bg * match_equity_after_loss(x_away, o_away, 3 * c)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_AWAY, match_equity, match_equity_after_loss, match_equity_after_win, position_equity,
    };
    use engine::probabilities::Probabilities;

    #[test]
    fn even_score_is_fifty_fifty() {
        for away in 1..=MAX_AWAY {
            assert_eq!(match_equity(away, away), 0.5);
        }
    }

    #[test]
    fn table_is_complementary() {
        // M[i][j] + M[j][i] == 1.0 for every pair of away scores.
        for i in 1..=MAX_AWAY {
            for j in 1..=MAX_AWAY {
                let sum = match_equity(i, j) + match_equity(j, i);
                assert!((sum - 1.0).abs() < 1e-6, "M({i},{j}) + M({j},{i}) = {sum}");
            }
        }
    }

    #[test]
    fn fewer_points_away_is_better() {
        // Being closer to winning never lowers the equity.
        for o_away in 1..=MAX_AWAY {
            for x_away in 2..=MAX_AWAY {
                assert!(match_equity(x_away - 1, o_away) >= match_equity(x_away, o_away));
            }
        }
    }

    #[test]
    fn known_values() {
        // Spot checks against Woolsey's published table.
        assert_eq!(match_equity(1, 1), 0.5);
        assert_eq!(match_equity(2, 4), 0.68);
        assert_eq!(match_equity(1, 2), 0.70);
        assert_eq!(match_equity(4, 2), 0.32);
    }

    #[test]
    fn already_won_or_lost() {
        assert_eq!(match_equity(0, 5), 1.0);
        assert_eq!(match_equity(5, 0), 0.0);
    }

    #[test]
    fn away_scores_are_clamped() {
        // Beyond the table we clamp to MAX_AWAY, so these must not panic.
        assert_eq!(match_equity(99, 99), 0.5);
        assert_eq!(match_equity(20, 3), match_equity(MAX_AWAY, 3));
    }

    #[test]
    fn winning_enough_points_wins_the_match() {
        // 2-away: winning 2+ points wins the match.
        assert_eq!(match_equity_after_win(2, 5, 2), 1.0);
        assert_eq!(match_equity_after_win(2, 5, 3), 1.0);
        // Winning a single point just improves the score.
        assert_eq!(match_equity_after_win(2, 5, 1), match_equity(1, 5));
    }

    #[test]
    fn losing_enough_points_loses_the_match() {
        assert_eq!(match_equity_after_loss(5, 2, 2), 0.0);
        assert_eq!(match_equity_after_loss(5, 2, 1), match_equity(5, 1));
    }

    fn switch_sides(p: &Probabilities) -> Probabilities {
        Probabilities {
            win_normal: p.lose_normal,
            win_gammon: p.lose_gammon,
            win_bg: p.lose_bg,
            lose_normal: p.win_normal,
            lose_gammon: p.win_gammon,
            lose_bg: p.win_bg,
        }
    }

    #[test]
    fn position_equity_is_antisymmetric_under_switch() {
        // The opponent's match-winning probability (their probs, their away
        // scores) must be exactly 1 minus ours. This is what makes move ranking
        // correct from either player's perspective.
        let p = Probabilities {
            win_normal: 0.25,
            win_gammon: 0.15,
            win_bg: 0.05,
            lose_normal: 0.35,
            lose_gammon: 0.1,
            lose_bg: 0.1,
        };
        for x_away in 1..=6 {
            for o_away in 1..=6 {
                let ours = position_equity(&p, x_away, o_away, 1);
                let theirs = position_equity(&switch_sides(&p), o_away, x_away, 1);
                assert!((ours + theirs - 1.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn match_ranking_can_differ_from_money() {
        // `a` wins fewer games but more of them are gammons, and it avoids a
        // gammon loss; `b` wins more games, all single, with a plain loss.
        let a = Probabilities {
            win_normal: 0.2,
            win_gammon: 0.4,
            win_bg: 0.0,
            lose_normal: 0.3,
            lose_gammon: 0.1,
            lose_bg: 0.0,
        };
        let b = Probabilities {
            win_normal: 0.6,
            win_gammon: 0.0,
            win_bg: 0.0,
            lose_normal: 0.4,
            lose_gammon: 0.0,
            lose_bg: 0.0,
        };
        // Money game values the gammons, so it prefers `a`.
        assert!(a.equity() > b.equity());
        // At 1-away/5-away a single win already wins the match, so the win
        // gammons are worthless; `b` wins more often and loses smaller, so match
        // play prefers `b` — the opposite ranking.
        assert!(position_equity(&b, 1, 5, 1) > position_equity(&a, 1, 5, 1));
    }
}
