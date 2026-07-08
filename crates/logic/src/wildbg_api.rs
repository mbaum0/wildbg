use crate::bg_move::BgMove;
use crate::cube::{CubeInfo, CubeState, MatchState};
use crate::match_equity::{MAX_AWAY, position_equity};
use engine::composite::CompositeEvaluator;
use engine::dice::Dice;
use engine::evaluator::Evaluator;
use engine::position::Position;
use engine::probabilities::Probabilities;

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
        self.evaluator
            .positions_and_probabilities(position, dice, config.value())
    }

    #[inline]
    pub fn best_move(&self, position: &Position, dice: &Dice, config: &ScoreConfig) -> BgMove {
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

#[cfg(test)]
mod tests {
    use crate::bg_move::{BgMove, MoveDetail};
    use crate::wildbg_api::{ScoreConfig, WildbgApi};
    use engine::dice::Dice;
    use engine::evaluator::EvaluatorFake;
    use engine::pos;
    use engine::position::Position;

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
