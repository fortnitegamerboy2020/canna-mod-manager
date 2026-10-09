//! Instant games share the arcade transaction, limits, payout factor and replay key.
use super::*;
use rand::seq::SliceRandom;

pub(super) const PLINKO_LOW: [i64; 13] = [
    200, 150, 120, 110, 100, 100, 80, 100, 100, 110, 120, 150, 200,
];
pub(super) const PLINKO_HIGH: [i64; 13] = [
    50000, 5000, 1000, 300, 100, 50, 20, 50, 100, 300, 1000, 5000, 50000,
];
pub(super) const WHEEL: [i64; 20] = [0, 1, 0, 2, 0, 1, 0, 5, 0, 1, 0, 2, 0, 1, 0, 10, 0, 1, 0, 1];

fn banker_draw(total: u8, player_third: Option<u8>) -> bool {
    match player_third {
        None => total <= 5,
        Some(card) => match total {
            0..=2 => true,
            3 => card != 8,
            4 => (2..=7).contains(&card),
            5 => (4..=7).contains(&card),
            6 => (6..=7).contains(&card),
            _ => false,
        },
    }
}
fn baccarat(rng: &mut impl Rng) -> (Vec<u8>, Vec<u8>, u8, u8) {
    // Fresh eight-deck shoe per round; ranks 10/J/Q/K are worth zero.
    let mut shoe: Vec<u8> = (0..8)
        .flat_map(|_| (1..=13).flat_map(|r| [if r >= 10 { 0 } else { r }; 4]))
        .collect();
    shoe.shuffle(rng);
    let mut player = vec![shoe.pop().unwrap(), shoe.pop().unwrap()];
    let mut banker = vec![shoe.pop().unwrap(), shoe.pop().unwrap()];
    let total = |cards: &[u8]| cards.iter().sum::<u8>() % 10;
    if total(&player) < 8 && total(&banker) < 8 {
        let third = if total(&player) <= 5 {
            let card = shoe.pop().unwrap();
            player.push(card);
            Some(card)
        } else {
            None
        };
        if banker_draw(total(&banker), third) {
            banker.push(shoe.pop().unwrap());
        }
    }
    let p = total(&player);
    let b = total(&banker);
    (player, banker, p, b)
}

fn fractional_return(stake: i64, hundredths: i64, factor: i64) -> i64 {
    whole_return(stake as i128 * hundredths as i128 * factor as i128 / 10000)
}
pub(super) fn result(
    input: &ArcadeInput,
    factor: i64,
    rng: &mut impl Rng,
) -> ApiResult<(i64, Value)> {
    match input.game.as_str() {
        "keno" => {
            let picks = input
                .picks
                .as_ref()
                .ok_or_else(|| bad("Pick four unique Keno numbers from 1 to 40"))?;
            let unique: std::collections::HashSet<_> = picks.iter().collect();
            if picks.len() != 4
                || unique.len() != 4
                || picks.iter().any(|n| !(1..=40).contains(n))
                || input.choice.is_some()
            {
                return Err(bad("Pick four unique Keno numbers from 1 to 40"));
            }
            let mut balls: Vec<i64> = (1..=40).collect();
            balls.shuffle(rng);
            balls.truncate(10);
            balls.sort_unstable();
            let hits = picks.iter().filter(|n| balls.contains(n)).count();
            let multiplier = [0, 0, 1, 5, 50][hits];
            Ok((
                whole_return(input.stake as i128 * multiplier as i128 * factor as i128 / 100),
                json!({"picks":picks,"draw":balls,"hits":hits,"base_multiplier":multiplier}),
            ))
        }
        "plinko" => {
            let risk = input.choice.as_deref().unwrap_or("low");
            if !matches!(risk, "low" | "high") || input.picks.is_some() {
                return Err(bad("Choose low or high Plinko risk"));
            }
            let path: Vec<u8> = (0..12).map(|_| rng.gen_range(0..2)).collect();
            let slot = path.iter().map(|n| *n as usize).sum::<usize>();
            let table = if risk == "high" {
                PLINKO_HIGH
            } else {
                PLINKO_LOW
            };
            Ok((
                fractional_return(input.stake, table[slot], factor),
                json!({"path":path,"slot":slot,"risk":risk,"multipliers":table.map(|n| n as f64/100.0),"base_multiplier":table[slot] as f64/100.0}),
            ))
        }
        "wheel" => {
            if input.choice.is_some() || input.picks.is_some() {
                return Err(bad("Wheel accepts only a stake"));
            }
            let slot = rng.gen_range(0..WHEEL.len());
            Ok((
                whole_return(input.stake as i128 * WHEEL[slot] as i128 * factor as i128 / 100),
                json!({"slot":slot,"segments":WHEEL,"base_multiplier":WHEEL[slot]}),
            ))
        }
        "baccarat" => {
            let choice = input.choice.as_deref().unwrap_or("");
            if !matches!(choice, "player" | "banker" | "tie") || input.picks.is_some() {
                return Err(bad("Bet on Player, Banker or Tie"));
            }
            let (player, banker, p, b) = baccarat(rng);
            let winner = if p == b {
                "tie"
            } else if p > b {
                "player"
            } else {
                "banker"
            };
            let push = winner == "tie" && choice != "tie";
            let paid = if push {
                input.stake
            } else if winner != choice {
                0
            } else {
                fractional_return(
                    input.stake,
                    match choice {
                        "banker" => 195,
                        "tie" => 900,
                        _ => 200,
                    },
                    factor,
                )
            };
            Ok((
                paid,
                json!({"player":player,"banker":banker,"player_total":p,"banker_total":b,"winner":winner,"choice":choice,"push":push}),
            ))
        }
        _ => Err(bad("Unknown arcade game")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    #[test]
    fn low_plinko_only_center_loses_at_base_factor_and_expected_return_is_bounded() {
        let mut losing_paths = 0;
        let mut returns = 0;
        for path in 0_u16..4096 {
            let slot = path.count_ones() as usize;
            losing_paths += usize::from(PLINKO_LOW[slot] < 100);
            returns += PLINKO_LOW[slot];
        }
        assert_eq!(losing_paths, 924); // 22.5586%, rather than the old 61.23%.
        assert_eq!(returns, 399560); // 97.5488% before rounding / owner factor.
        assert_eq!(fractional_return(25, PLINKO_LOW[6], 100), 20);
    }
    #[test]
    fn banker_third_card_table_and_naturals() {
        for card in 0..10 {
            assert!(banker_draw(2, Some(card)));
            assert_eq!(banker_draw(3, Some(card)), card != 8);
            assert_eq!(banker_draw(4, Some(card)), (2..=7).contains(&card));
            assert_eq!(banker_draw(5, Some(card)), (4..=7).contains(&card));
            assert_eq!(banker_draw(6, Some(card)), (6..=7).contains(&card));
            assert!(!banker_draw(7, Some(card)));
        }
        let mut rng = StdRng::seed_from_u64(31);
        for _ in 0..2000 {
            let (p, b, pt, bt) = baccarat(&mut rng);
            assert_eq!(pt, p.iter().sum::<u8>() % 10);
            assert_eq!(bt, b.iter().sum::<u8>() % 10);
            assert!((2..=3).contains(&p.len()) && (2..=3).contains(&b.len()));
            if (p[0] + p[1]) % 10 >= 8 || (b[0] + b[1]) % 10 >= 8 {
                assert_eq!((p.len(), b.len()), (2, 2));
            }
        }
    }
    #[test]
    fn new_games_validate_inputs_and_results() {
        let mut rng = StdRng::seed_from_u64(82);
        for game in ["keno", "plinko", "wheel", "baccarat"] {
            let input: ArcadeInput=serde_json::from_value(json!({"request_id":"x","game":game,"stake":100,"choice":if game=="baccarat" {Some("banker")} else if game=="plinko" {Some("high")} else {None},"picks":if game=="keno" {Some(vec![1,2,3,4])} else {None}})).unwrap();
            for _ in 0..100 {
                let (paid, v) = result(&input, 100, &mut rng).unwrap();
                assert!((0..=50000).contains(&paid));
                if game == "keno" {
                    let draw = v["draw"].as_array().unwrap();
                    assert_eq!(draw.len(), 10);
                    assert!(draw.windows(2).all(|p| p[0].as_i64() < p[1].as_i64()));
                }
                if game == "plinko" {
                    assert_eq!(v["path"].as_array().unwrap().len(), 12);
                }
            }
        }
        let bad_input = serde_json::from_value(
            json!({"request_id":"x","game":"keno","stake":10,"picks":[1,1,2,3]}),
        )
        .unwrap();
        assert!(result(&bad_input, 100, &mut rng).is_err());
        assert_eq!(fractional_return(3, 50, 150), 2);
        assert_eq!(fractional_return(3, 195, 125), 7);
    }
}
