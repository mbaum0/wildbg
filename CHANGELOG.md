# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

As long as the mayor version is `0`, the API is not stable and may change with minor versions.

The minor version will be incremented if any of the following changes:

- The HTTP API
- The C API
- The inputs for the neural networks

This means you can reuse the same neural networks between for example 0.2.0 and 0.2.1, but not between 0.1.0 and 0.2.0.

## Unreleased

### Added

- Money game cube decisions are now based on Janowski's cube formulae instead of a placeholder. The `/eval` HTTP endpoint and the C `cube_info` function additionally return cubeful equities (`cubelessEquity`, `equityNoDouble`, `equityDoubleTake`) and accept the current cube position (centered, owned, opponent-owned) via `cube_position`: ([#17](https://github.com/carsten-wenderdel/wildbg/issues/17))
- Match play cube decisions using a live-cube model (recursive take points, capturing recube vig and cube ownership) on top of Kit Woolsey's match equity table, including Crawford and post-Crawford handling. The `/eval` endpoint gains `x_away`/`o_away`/`crawford`/`cube_value` parameters, and the C `cube_info` function gains a `cube_value` argument and a `BgConfig` (with a new `crawford` field): ([#17](https://github.com/carsten-wenderdel/wildbg/issues/17))
- Match play checker decisions: `best_move` and the `/move` endpoint now rank moves by match-winning probability (via the match equity table) at arbitrary match scores, instead of always using money game equity: ([#17](https://github.com/carsten-wenderdel/wildbg/issues/17))
- New C function `ranked_moves` returns every legal move ranked best-first with the value it is ranked by (`CRankedMove`), so callers can weaken play by picking a near-best move without re-evaluating positions themselves. `out[0]` matches `best_move`.
- In a race, checker play now breaks ties by the fewest expected rolls left to bear off, so a won
  race is finished promptly instead of dawdling. Ranking by win probability alone is *flat* once a
  race is decided -- winning in four rolls and winning in seven are both worth one point -- so every
  legal move scored the same and `sort_unstable_by` picked among them arbitrarily. Equity remains the
  primary key, so gammon saving still wins wherever it genuinely differs. New module
  `engine::race` supplies the exact figure from a memoised dynamic program over bear-off positions,
  cross-checked against `Position::all_positions_after_moving` in its tests; positions with a checker
  still outside the home board sort behind every bear-off position, ordered by pip count.

### Changed

- Replaced the committed neural nets with far stronger ones from
  [wildbg-training](https://github.com/carsten-wenderdel/wildbg-training) (`contact.onnx` from
  `data/0020`, `race.onnx` from `data/0022`). Both are the three-hidden-layer 300/250/200 models,
  replacing a 202-150-6 contact net and a 186-16-6 race net. Measured over 101,200 duel games with
  `compare-evaluators`, the new pair wins by **0.465 equity per game**. The input encodings are
  unchanged (202 contact / 186 race inputs), so this is a pure weights swap.

  `data/0020` is used for contact rather than the newer `data/0021`: the two are statistically
  indistinguishable in strength (0020 by 0.003 +/- 0.004 equity over 85,700 games), but 0021 fails
  `player_runs_in_money_game_but_not_in_1ptr` -- it leaves a back checker trapped instead of running
  it home to save a gammon, and rates that unwinnable position at a 16.9% win probability.
- The neural-net quality tests in `onnx.rs` now evaluate their positions with the *race* net.
  Every position they use is a race (`game_phase()` returns `Ongoing(Race)`), so `CompositeEvaluator`
  routes them to the race net in production; they had been asserting contact-net output on
  positions the contact net is never asked about and is not trained for. A new test guards that
  premise. Two of them assert only `gammon + backgammon` rather than the split, which the nets
  get wrong on 15-checkers-on-one-pip positions.
- `composite.rs`'s `game_over_ongoing` test now uses a legal position. It had used one with 13
  checkers borne off while two remained on pip 23, which cannot occur (bearing off requires every
  checker to be home), so the equity bounds it asserted were reading extrapolated nonsense.

### Fixed

- When converting data for training with PyTorch, the winning-gammon values erroneously also included the backgammon values: ([#40](https://github.com/carsten-wenderdel/wildbg/issues/40))
- Removed duplicates in the move generation for forced bear offs.

### Internal / Training

- Finding positions for training new nets is based on weaknesses of the older nets.
- Training data generation can be suspended and resumed.
- Finding the best neural net by running a multi-arm-bandit inspired competition using UCB.

## 0.3.0 - 2025-08-06

Thanks for their contributions:

- [@deprus](https://github.com/deprus) ([#29](https://github.com/carsten-wenderdel/wildbg/issues/29))
- [@mbaum0](https://github.com/mbaum0) ([#28](https://github.com/carsten-wenderdel/wildbg/pull/28), [#30](https://github.com/carsten-wenderdel/wildbg/pull/30), [#32](https://github.com/carsten-wenderdel/wildbg/pull/32))
- [@OfirMarom](https://github.com/OfirMarom) ([#27](https://github.com/carsten-wenderdel/wildbg/issues/27))
- [@th3oth3rjak3](https://github.com/th3oth3rjak3) ([#25](https://github.com/carsten-wenderdel/wildbg/pull/25))
- [@macherius](https://github.com/macherius) ([#24](https://github.com/carsten-wenderdel/wildbg/pull/24))

### Added

- C and HTTP API support 1-pointers (before only money game).
- The C API supports cube actions.
- HTTP address and port are configurable for the HTTP API.
- Use custom allocator `MiMalloc` for faster and consistent memory handling.

### Fixed

- In edge cases some mixed moves where not calculated
  correctly: ([#29](https://github.com/carsten-wenderdel/wildbg/issues/29))
- Position IDs were encoded wrongly when the opponent had checkers on the
  bar: ([#27](https://github.com/carsten-wenderdel/wildbg/issues/27))

### Changed

- Neural nets inputs generation is faster: 640% more calculations in the same time for race inputs,
  760% more calculations for contact inputs.
- Move generation is faster: From 109% more calculations in the same time (mixed moves, contact) to 160% more
  (mixed moves, race).
- Breaking change: The C API returns an array of move details instead of four separate fields.
- Default neural nets are now compiled into the executable.

### Internal / Training

- Split training scripts for `race` and `contact` and go back to `ReLU` for `race` positions.
- Use `CrossEntropyLoss` as PyTorch Optimizer and add `softmax` only after training.
- Rollouts and the training process are now deterministic.
- Generation of multiple neural nets during one training process and better tools for comparison/benchmarking.

## 0.2.0 - 2023-11-26

### Added

- The C API now supports raw evaluation of positions.
- Big speed up by batch inference of neural networks.

### Changed

- The C API doesn't need to reload the neural nets for every call.
- Different neural networks for _contact_ and _race_.

### Internal / Training

- Documentation for `engine` and the training process.
- Use `L1Loss` instead of `MSELoss` as loss function during supervised training.
- Use `AdamW` instead of `SGD` as PyTorch Optimizer.
- Use `Hardsigmoid` instead of `ReLU` for hidden layers.
- Rollout data is now stored with GnuBG position IDs.
- Improved selection of positions for rollouts via self play.

## 0.1.0 - 2023-10-17

Initial release of `wildbg`.

Thanks for their contributions:

- [@bungogood](https://github.com/bungogood) ([#10](https://github.com/carsten-wenderdel/wildbg/pull/10), [11](https://github.com/carsten-wenderdel/wildbg/pull/11))
- [@oradwastaken](https://github.com/oradwastaken) ([#7](https://github.com/carsten-wenderdel/wildbg/pull/7), [#9](https://github.com/carsten-wenderdel/wildbg/pull/9))

### Added

- Simple C API for best move in 1-pointer.
- HTTP API for moves and cubes.
- Inference of existing neural networks with [`tract`](https://github.com/sonos/tract).
- Move generation.

### Internal / Training

- Implementation of the GnuBG position ID.
- Comparison of neural networks by playing against each other.
- Training of new neural networks with `PyTorch`.
- Rollouts with fixed number of 1296 games for generating training data.
- Finding positions for rollouts via self play.
