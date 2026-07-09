#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>


typedef struct Wildbg Wildbg;

/**
 * If the move is not possible, both `from` and `to` will contain `-1`.
 *
 * If the move is possible, `from` is an integer between 25 and 1,
 * `to` is an integer between 24 and 0.
 * `from - to` is then at least 1 and at most 6.
 */
typedef struct CMoveDetail {
  int from;
  int to;
} CMoveDetail;

/**
 * When no move is possible, detail_count will be 0.
 *
 * If only a single move is possible, `details[0]` will contain this information.
 * `detail_count` will contain a value between 0 and 4.
 *
 * If the same checker is moved twice, this is encoded in two details.
 */
typedef struct CMove {
  struct CMoveDetail details[4];
  int detail_count;
} CMove;

/**
 * Configuration needed for the evaluation of positions.
 *
 * Both checker play (`best_move`) and the cube decision (`cube_info`) support
 * money game (`x_away == 0 && o_away == 0`) and arbitrary match scores.
 */
typedef struct BgConfig {
  /**
   * Number of points the player on turn needs to finish the match. Zero indicates money game.
   */
  unsigned int x_away;
  /**
   * Number of points the opponent needs to finish the match. Zero indicates money game.
   */
  unsigned int o_away;
  /**
   * Whether the current game is the Crawford game. Only relevant for the cube decision in match play.
   */
  bool crawford;
} BgConfig;

typedef struct CCubeInfo {
  bool should_double;
  bool should_accept;
  float cubeless_equity;
  float equity_no_double;
  float equity_double_take;
} CCubeInfo;

typedef struct CProbabilities {
  /**
   * Cubeless probability to win the game. This includes gammons and backgammons.
   */
  float win;
  /**
   * Probability to win gammon or backgammon.
   */
  float win_g;
  /**
   * Probability to win backgammon.
   */
  float win_bg;
  /**
   * Probability to lose gammon or backgammon.
   */
  float lose_g;
  /**
   * Probability to lose backgammon.
   */
  float lose_bg;
} CProbabilities;

/**
 * A legal move together with the value it is ranked by. Returned by
 * `ranked_moves`, best-first.
 */
typedef struct CRankedMove {
  /**
   * The move itself, in the same encoding as the return value of `best_move`.
   */
  struct CMove checker_move;
  /**
   * The value the move is ranked by: cubeless win probability for a
   * 1-pointer, cubeless equity for a money game, and cubeless match-winning
   * probability for match play. Higher is better.
   */
  float value;
} CRankedMove;

/**
 * Returns the best move for the given position.
 *
 * The player on turn always moves from pip 24 to pip 1.
 * The array `pips` contains the player's bar in index 25, the opponent's bar in index 0.
 * Checkers of the player on turn are encoded with positive integers, the opponent's checkers with negative integers.
 *
 * # Safety
 * The argument `wildbg` needs to be initialized with `wildbg_new()` and `wildbg_free()` must not be called yet.
 * Otherwise we have random memory access here.
 */
struct CMove best_move(const struct Wildbg *wildbg,
                       const int (*pips)[26],
                       unsigned int die1,
                       unsigned int die2,
                       const struct BgConfig *config);

/**
 * Returns the money game cube decision for a certain position.
 * If an illegal position is encountered, all values will be zero/false.
 *
 * `cube_position` describes who currently owns the doubling cube, from the
 * player on turn's perspective: `0` for a centered cube (an initial double
 * decision), `1` if the player on turn owns the cube, `-1` if the opponent
 * owns it. Any other value is treated as a centered cube.
 *
 * `cube_value` is the current value of the doubling cube (1, 2, 4, …). Values
 * below 1 are treated as 1. It only affects match play.
 *
 * `config` supplies the match score: `x_away`/`o_away` of `0` (both) means a
 * money game, otherwise it is match play at that score, honouring `crawford`.
 *
 * The player on turn always moves from pip 24 to pip 1.
 * The array `pips` contains the player's bar in index 25, the opponent's bar in index 0.
 * Checkers of the player on turn are encoded with positive integers, the opponent's checkers with negative integers.
 */
struct CCubeInfo cube_info(const struct Wildbg *wildbg,
                           const int (*pips)[26],
                           int cube_position,
                           int cube_value,
                           const struct BgConfig *config);

/**
 * Returns cubeless money game probabilities for a certain position.
 * If an illegal position is encountered, all probabilities will be zero.
 *
 * The player on turn always moves from pip 24 to pip 1.
 * The array `pips` contains the player's bar in index 25, the opponent's bar in index 0.
 * Checkers of the player on turn are encoded with positive integers, the opponent's checkers with negative integers.
 */
struct CProbabilities probabilities(const struct Wildbg *wildbg,
                                    const int (*pips)[26]);

/**
 * Fills `out` with the legal moves for the given position, ranked best-first,
 * and returns how many were written (never more than `max_moves`).
 *
 * Each entry pairs the move (encoded as in `best_move`) with the value it is
 * ranked by, so a caller can weaken play by choosing a move whose value is
 * close to the best instead of always the best one. `out[0]` is the same move
 * `best_move` would return. Returns `0` and writes nothing for an illegal
 * position, invalid dice, or `max_moves <= 0`.
 *
 * The board and score conventions match `best_move`.
 *
 * # Safety
 * The argument `wildbg` needs to be initialized with `wildbg_new()` and `wildbg_free()` must not be called yet.
 * `out` must point to at least `max_moves` writable `CRankedMove` values.
 */
int ranked_moves(const struct Wildbg *wildbg,
                 const int (*pips)[26],
                 unsigned int die1,
                 unsigned int die2,
                 const struct BgConfig *config,
                 struct CRankedMove *out,
                 int max_moves);

/**
 * # Safety
 *
 * Frees the memory of the argument.
 * Don't call it with a NULL pointer. Don't call it more than once for the same `Wildbg` pointer.
 */
void wildbg_free(struct Wildbg *ptr);

/**
 * Loads the neural nets into memory and returns a pointer to the API.
 * Returns `NULL` if the neural nets cannot be found.
 *
 * To free the memory after usage, call `wildbg_free`.
 */
struct Wildbg *wildbg_new(void);
