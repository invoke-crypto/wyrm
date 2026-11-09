//! 256-bit intermediates for products of token amounts, which overflow u128 (1e24 yocto times 1e27 WYRM units).

/// `a * b` as (high, low) 128-bit halves.
pub fn wide_mul(a: u128, b: u128) -> (u128, u128) {
    const LO: u128 = u64::MAX as u128;
    let (a1, a0) = (a >> 64, a & LO);
    let (b1, b0) = (b >> 64, b & LO);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & LO) + (p10 & LO);
    let lo = (p00 & LO) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    (hi, lo)
}

/// `floor(a * b / d)`, saturating at `u128::MAX` when the quotient does not fit. `d` must be positive.
pub fn mul_div(a: u128, b: u128, d: u128) -> u128 {
    assert!(d > 0, "division by zero");
    let (hi, lo) = wide_mul(a, b);
    if hi == 0 {
        return lo / d;
    }
    if hi >= d {
        return u128::MAX;
    }
    let mut rem = hi;
    let mut q = 0u128;
    for i in (0..128).rev() {
        let carry = rem >> 127;
        rem = (rem << 1) | ((lo >> i) & 1);
        q <<= 1;
        if carry == 1 || rem >= d {
            rem = rem.wrapping_sub(d);
            q |= 1;
        }
    }
    q
}

/// `a * b >= c * d` without overflow.
pub fn mul_ge(a: u128, b: u128, c: u128, d: u128) -> bool {
    wide_mul(a, b) >= wide_mul(c, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn mul_div_matches_exact_math_when_the_product_fits(
            a in any::<u64>(),
            b in any::<u64>(),
            d in 1u128..u128::MAX,
        ) {
            let (a, b) = (a as u128, b as u128);
            prop_assert_eq!(mul_div(a, b, d), a * b / d);
        }

        #[test]
        fn mul_div_undoes_a_multiplication_past_u128(a in any::<u128>(), b in 1u128..u128::MAX) {
            // a * b / b == a, with a product up to 2^256.
            prop_assert_eq!(mul_div(a, b, b), a);
        }

        #[test]
        fn mul_ge_agrees_with_exact_math(a in any::<u64>(), b in any::<u64>(), c in any::<u64>(), d in any::<u64>()) {
            let (a, b, c, d) = (a as u128, b as u128, c as u128, d as u128);
            prop_assert_eq!(mul_ge(a, b, c, d), a * b >= c * d);
        }
    }

    #[test]
    fn mul_div_saturates_instead_of_wrapping() {
        assert_eq!(mul_div(u128::MAX, u128::MAX, 1), u128::MAX);
        assert_eq!(mul_div(u128::MAX, 4, 2), u128::MAX);
    }
}
