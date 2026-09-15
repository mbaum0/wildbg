use crate::bg_move::BgMove;
use crate::cube::{CubeInfo, CubeState, MatchState};
use crate::match_equity::{MAX_AWAY, position_equity};
use engine::composite::CompositeEvaluator;
use engine::dice::Dice;
use engine::evaluator::Evaluator;
use engine::position::{GamePhase, OngoingPhase, Position};
use engine::probabilities::Probabilities;
use engine::race::expected_rolls;

pub enum ScoreConfig {
    MoneyGame,
    OnePointer,
    /// Match play at the given away score, ranking moves by match-winning probability.
    Match {
        x_away: u32,
        o_away: u32,
    },
}

impl TryFrom<(u32, u32)> for ScoreConfig {
    type Error = &'static str;

    #[inline]
    fn try_from((x_away, o_away): (u32, u32)) -> Result<Self, Self::Error> {
        match (x_away, o_away) {
            (0, 0) => Ok(ScoreConfig::MoneyGame),
            (1, 1) => Ok(ScoreConfig::OnePointer),
            (0, _) | (_, 0) => Err("For match play both x_away and o_away must be at least 1."),
            (x_away, o_away) if x_away <= MAX_AWAY && o_away <= MAX_AWAY => {
                Ok(ScoreConfig::Match { x_away, o_away })
            }
            (_, _) => Err("Away scores larger than the match equity table are not supported."),
        }
    }
}

impl ScoreConfig {
    #[inline]
    pub fn value(&self) -> Box<dyn Fn(&Probabilities) -> f32> {
        match *self {
            ScoreConfig::OnePointer => Box::new(|p: &Probabilities| p.win()),
            ScoreConfig::MoneyGame => Box::new(|p: &Probabilities| p.equity()),
            // Rank moves by cubeless match-winning probability at the base cube.
            ScoreConfig::Match { x_away, o_away } => {
                Box::new(move |p: &Probabilities| position_equity(p, x_away, o_away, 1))
            }
        }
    }
}

pub struct WildbgApi<T: Evaluator> {
    evaluator: T,
}

impl WildbgApi<CompositeEvaluator> {
    pub fn try_default() -> Result<Self, String> {
        CompositeEvaluator::try_default().map(|evaluator| Self { evaluator })
    }
}

impl<T: Evaluator> WildbgApi<T> {
    #[inline]
    pub fn with_evaluator(evaluator: T) -> Self {
        Self { evaluator }
    }

    #[inline]
    pub fn probabilities(&self, position: &Position) -> Probabilities {
        self.evaluator.eval(position)
    }

    #[inline]
    pub fn all_moves(
        &self,
        position: &Position,
        dice: &Dice,
        config: &ScoreConfig,
    ) -> Vec<(Position, Probabilities)> {
        let value = config.value();
        let mut moves = self
            .evaluator
            .positions_and_probabilities(position, dice, &value);
        if is_race(position) {
            order_race_moves(&mut moves, &value);
        }
        moves
    }

    #[inline]
    pub fn best_move(&self, position: &Position, dice: &Dice, config: &ScoreConfig) -> BgMove {
        // In a race, go through `all_moves` so the move inherits the expected-rolls
        // ordering below. Elsewhere `best_position` is cheaper, keeping its shortcuts.
        if is_race(position) {
            let moves = self.all_moves(position, dice, config);
            let (best, _) = moves
                .first()
                .expect("there is always at least one legal move");
            // `all_moves` already reports positions from the mover's point of view.
            return BgMove::new(position, best, dice);
        }
        let new_position = self.evaluator.best_position(position, dice, config.value());
        BgMove::new(position, &new_position.sides_switched(), dice)
    }

    pub fn cube_info(
        &self,
        position: &Position,
        cube: CubeState,
        match_state: MatchState,
    ) -> CubeInfo {
        CubeInfo::for_state(&self.evaluator.eval(position), cube, match_state)
    }
}

/// Two moves count as equally good when their values differ by less than this.
///
/// Exact comparison would not do: the nets end in a softmax, which never emits a
/// clean 1.0, so a decided race yields values that are merely very close rather
/// than identical. At the same time this is small enough to be far below the nets'
/// own noise, and orders of magnitude below a real difference such as saving a
/// gammon, so equity still decides whenever it genuinely differs.
const RACE_TIE_EPSILON: f32 = 1e-4;

#[inline]
fn is_race(position: &Position) -> bool {
    matches!(
        position.game_phase(),
        GamePhase::Ongoing(OngoingPhase::Race)
    )
}

/// Re-order race moves by value first and fewest expected rolls second.
///
/// Once a race is decided every legal move has the same value -- a four-roll win and
/// a seven-roll win both score one point -- so ranking by value alone leaves the
/// choice to `sort_unstable_by`, which among equal values keeps whichever move
/// happened to land first. That is what makes the engine dawdle over a won race
/// instead of finishing it. Value stays the primary key, so this only decides moves
/// that are genuinely equivalent on equity.
fn order_race_moves<F>(moves: &mut Vec<(Position, Probabilities)>, value: &F)
where
    F: Fn(&Probabilities) -> f32 + ?Sized,
{
    // Decorate first: expected_rolls is memoised but still worth not recomputing
    // inside every comparison.
    let mut keyed: Vec<(f32, f32, Position, Probabilities)> = moves
        .drain(..)
        .map(|(position, probabilities)| {
            let bucket = (value(&probabilities) / RACE_TIE_EPSILON).round();
            (bucket, expected_rolls(&position), position, probabilities)
        })
        .collect();
    keyed.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.total_cmp(&b.1)));
    moves.extend(
        keyed
            .into_iter()
            .map(|(_, _, position, probabilities)| (position, probabilities)),
    );
}

#[cfg(test)]
mod tests {
    use crate::bg_move::{BgMove, MoveDetail};
    use crate::wildbg_api::{ScoreConfig, WildbgApi};
    use engine::dice::Dice;
    use engine::evaluator::EvaluatorFake;
    use engine::pos;
    use engine::position::Position;
    use engine::race::expected_rolls;

    /// x has one checker on the six point and one on the two point, thirteen already
    /// off. o is stuck on the far side needing about ninety pips, so the race is long
    /// since decided. With 6-2, x can take both checkers off and finish now, or play
    /// 6/4 with the two and bear that checker off with the six, leaving one behind.
    /// Both plays use both dice, so only expected rolls separates them.
    fn decided_race() -> Position {
        pos![x 6:1, 2:1; o 19:15]
    }

    /// Every position evaluates identically, so the value key cannot order anything
    /// and the expected-rolls tie-break is the only thing left that can.
    fn flat_evaluator() -> EvaluatorFake {
        EvaluatorFake::with_default([0.5, 0.0, 0.0, 0.5, 0.0, 0.0].into())
    }

    #[test]
    fn race_moves_are_ordered_by_quickest_finish() {
        let api = WildbgApi {
            evaluator: flat_evaluator(),
        };

        let moves = api.all_moves(&decided_race(), &Dice::new(6, 2), &ScoreConfig::MoneyGame);

        assert!(moves.len() > 1, "the position should offer a real choice");
        // Finishing outright needs no further rolls; anything else needs at least one.
        assert_eq!(expected_rolls(&moves[0].0), 0.0);
        assert!(expected_rolls(&moves[1].0) > 0.0);
        // And the whole list is sorted, not merely the winner.
        for pair in moves.windows(2) {
            assert!(expected_rolls(&pair[0].0) <= expected_rolls(&pair[1].0));
        }
    }

    #[test]
    fn best_move_in_a_decided_race_finishes_at_once() {
        let api = WildbgApi {
            evaluator: flat_evaluator(),
        };

        let bg_move = api.best_move(&decided_race(), &Dice::new(6, 2), &ScoreConfig::MoneyGame);

        // Both checkers come off: 6 bears off with the six, 2 bears off with the two.
        // `to: 0` is the tray.
        assert_eq!(
            bg_move,
            BgMove {
                details: vec![MoveDetail { from: 6, to: 0 }, MoveDetail { from: 2, to: 0 }],
            }
        );
    }

    #[test]
    fn contact_positions_keep_their_value_ordering() {
        // Contact play must be untouched: the order has to match what the evaluator
        // produces on its own, with no expected-rolls term mixed in.
        use engine::evaluator::Evaluator;
        use engine::position::STARTING;

        let position = STARTING;
        assert!(
            !super::is_race(&position),
            "STARTING must be a contact position"
        );
        let dice = Dice::new(4, 2);
        let config = ScoreConfig::MoneyGame;

        let expected: Vec<Position> = flat_evaluator()
            .positions_and_probabilities(&position, &dice, config.value())
            .into_iter()
            .map(|(position, _)| position)
            .collect();
        let api = WildbgApi {
            evaluator: flat_evaluator(),
        };
        let actual: Vec<Position> = api
            .all_moves(&position, &dice, &config)
            .into_iter()
            .map(|(position, _)| position)
            .collect();

        assert!(actual.len() > 1);
        assert_eq!(actual, expected);
    }

    fn position_with_lowest_equity() -> Position {
        pos!(x 5:1, 3:1; o 20:2).sides_switched()
    }

    /// Test double. Returns not so good probabilities for `expected_pos`, better for everything else.
    fn evaluator_fake() -> EvaluatorFake {
        let mut fake = EvaluatorFake::with_default([0.38, 0.2, 0.1, 0.12, 0.1, 0.1].into());
        fake.insert(
            position_with_lowest_equity(),
            [0.5, 0.1, 0.1, 0.1, 0.1, 0.1].into(),
        );
        fake
    }

    #[test]
    fn best_move_1ptr() {
        // Given
        let given_pos = pos!(x 7:2; o 20:2);
        let evaluator = evaluator_fake();
        let api = WildbgApi { evaluator };
        // When
        let config = ScoreConfig::OnePointer;
        let bg_move = api.best_move(&given_pos, &Dice::new(4, 2), &config);
        // Then
        let expected_move = BgMove {
            details: vec![MoveDetail { from: 7, to: 5 }, MoveDetail { from: 5, to: 1 }],
        };
        assert_eq!(bg_move, expected_move);
    }

    #[test]
    fn best_move_money_game() {
        // Given
        let given_pos = pos!(x 7:2; o 20:2);
        let evaluator = evaluator_fake();
        let api = WildbgApi { evaluator };
        // When
        let config = ScoreConfig::MoneyGame;
        let bg_move = api.best_move(&given_pos, &Dice::new(4, 2), &config);
        // Then
        let expected_move = BgMove {
            details: vec![MoveDetail { from: 7, to: 3 }, MoveDetail { from: 7, to: 5 }],
        };
        assert_eq!(bg_move, expected_move);
    }

    #[test]
    fn best_move_match() {
        // Given a deep, symmetric match score, where match equity is close to
        // linear, the match move should match the money move for this fixture.
        let given_pos = pos!(x 7:2; o 20:2);
        let evaluator = evaluator_fake();
        let api = WildbgApi { evaluator };
        // When
        let config = ScoreConfig::try_from((7, 7)).unwrap();
        assert!(matches!(config, ScoreConfig::Match { .. }));
        let bg_move = api.best_move(&given_pos, &Dice::new(4, 2), &config);
        // Then
        let expected_move = BgMove {
            details: vec![MoveDetail { from: 7, to: 3 }, MoveDetail { from: 7, to: 5 }],
        };
        assert_eq!(bg_move, expected_move);
    }
}
