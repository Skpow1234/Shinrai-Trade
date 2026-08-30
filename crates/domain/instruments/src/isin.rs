//! ISO 6166 ISIN check-digit validation.

use crate::error::InstrumentError;

/// Validates an ISIN: 12 alphanumeric characters with a correct check digit.
///
/// # Errors
///
/// Returns [`InstrumentError::InvalidIsin`] if the value fails format or checksum.
pub fn validate_isin(isin: &str) -> Result<(), InstrumentError> {
    let isin = isin.trim();
    if isin.len() != 12 {
        return Err(InstrumentError::InvalidIsin);
    }
    if !isin.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(InstrumentError::InvalidIsin);
    }
    let upper = isin.to_ascii_uppercase();
    let body = &upper[..11];
    let check = upper.as_bytes()[11];
    let expected = check_digit(body).ok_or(InstrumentError::InvalidIsin)?;
    if check != expected {
        return Err(InstrumentError::InvalidIsin);
    }
    Ok(())
}

/// ISO 6166 check digit: expand letters to digits, double every second digit
/// from the right, sum digit values, check = (10 - (sum % 10)) % 10.
fn check_digit(body: &str) -> Option<u8> {
    let mut digits = Vec::with_capacity(22);
    for c in body.chars() {
        if c.is_ascii_digit() {
            digits.push(u8::try_from(c.to_digit(10)?).ok()?);
        } else if c.is_ascii_uppercase() {
            let n = u8::try_from(c as u32 - 55).ok()?; // A=10
            digits.push(n / 10);
            digits.push(n % 10);
        } else {
            return None;
        }
    }

    let mut sum = 0_u32;
    let mut double = true;
    for &d in digits.iter().rev() {
        let mut v = u32::from(d);
        if double {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
        double = !double;
    }
    let check = (10 - (sum % 10)) % 10;
    Some(b'0' + u8::try_from(check).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_apple_isin() {
        validate_isin("US0378331005").expect("AAPL ISIN");
    }

    #[test]
    fn rejects_bad_check_digit() {
        assert!(validate_isin("US0378331006").is_err());
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(validate_isin("US037833100").is_err());
    }
}
