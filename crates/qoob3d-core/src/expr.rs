//! Numeric expressions for value input (SK-IN-03, SK-IN-04): numbers,
//! `+ - * /`, parentheses, unary minus and length units. The result is in
//! millimetres. Parameter names follow with the solver (M3b).

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError {
    /// Byte offset in the input where the problem was found.
    pub at: usize,
    pub kind: ExprErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprErrorKind {
    Empty,
    UnexpectedChar(char),
    UnexpectedEnd,
    UnknownUnit(String),
    DivisionByZero,
    NotFinite,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} at {}", self.kind, self.at)
    }
}

impl std::error::Error for ExprError {}

/// Evaluates `input` to millimetres. `decimal_comma` additionally accepts
/// `,` as decimal separator (German UI); `.` is always accepted.
pub fn eval_length_mm(input: &str, decimal_comma: bool) -> Result<f64, ExprError> {
    let mut p = Parser { s: input.as_bytes(), pos: 0, decimal_comma };
    p.skip_ws();
    if p.pos == p.s.len() {
        return Err(p.err(ExprErrorKind::Empty));
    }
    let v = p.sum()?;
    p.skip_ws();
    if p.pos < p.s.len() {
        return Err(p.err(ExprErrorKind::UnexpectedChar(p.s[p.pos] as char)));
    }
    if !v.is_finite() {
        return Err(ExprError { at: 0, kind: ExprErrorKind::NotFinite });
    }
    Ok(v)
}

/// Like [`eval_length_mm`] but for plain numbers such as angles (no units).
pub fn eval_number(input: &str, decimal_comma: bool) -> Result<f64, ExprError> {
    eval_length_mm(input, decimal_comma)
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
    decimal_comma: bool,
}

impl Parser<'_> {
    fn err(&self, kind: ExprErrorKind) -> ExprError {
        ExprError { at: self.pos, kind }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.s.get(self.pos).copied()
    }

    fn sum(&mut self) -> Result<f64, ExprError> {
        let mut v = self.product()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.pos += 1;
            let rhs = self.product()?;
            v = if op == b'+' { v + rhs } else { v - rhs };
        }
        Ok(v)
    }

    fn product(&mut self) -> Result<f64, ExprError> {
        let mut v = self.unary()?;
        while let Some(op @ (b'*' | b'/')) = self.peek() {
            self.pos += 1;
            let at = self.pos;
            let rhs = self.unary()?;
            if op == b'/' {
                if rhs == 0.0 {
                    return Err(ExprError { at, kind: ExprErrorKind::DivisionByZero });
                }
                v /= rhs;
            } else {
                v *= rhs;
            }
        }
        Ok(v)
    }

    fn unary(&mut self) -> Result<f64, ExprError> {
        match self.peek() {
            Some(b'-') => {
                self.pos += 1;
                Ok(-self.unary()?)
            }
            Some(b'+') => {
                self.pos += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Result<f64, ExprError> {
        let v = match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                let v = self.sum()?;
                match self.peek() {
                    Some(b')') => self.pos += 1,
                    Some(c) => return Err(self.err(ExprErrorKind::UnexpectedChar(c as char))),
                    None => return Err(self.err(ExprErrorKind::UnexpectedEnd)),
                }
                v
            }
            Some(c) if c.is_ascii_digit() || c == b'.' || (self.decimal_comma && c == b',') => {
                self.number()?
            }
            Some(c) => return Err(self.err(ExprErrorKind::UnexpectedChar(c as char))),
            None => return Err(self.err(ExprErrorKind::UnexpectedEnd)),
        };
        Ok(v * self.unit()?)
    }

    fn number(&mut self) -> Result<f64, ExprError> {
        let start = self.pos;
        let mut text = String::new();
        while let Some(&c) = self.s.get(self.pos) {
            match c {
                b'0'..=b'9' => text.push(c as char),
                b'.' => text.push('.'),
                b',' if self.decimal_comma => text.push('.'),
                _ => break,
            }
            self.pos += 1;
        }
        text.parse::<f64>().map_err(|_| ExprError {
            at: start,
            kind: ExprErrorKind::UnexpectedChar(self.s[start] as char),
        })
    }

    /// Optional unit suffix → factor to millimetres.
    fn unit(&mut self) -> Result<f64, ExprError> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_alphabetic() {
            self.pos += 1;
        }
        let unit = std::str::from_utf8(&self.s[start..self.pos]).unwrap_or_default();
        match unit {
            "" | "mm" => Ok(1.0),
            "cm" => Ok(10.0),
            "m" => Ok(1000.0),
            "in" => Ok(25.4),
            "um" => Ok(0.001),
            other => Err(ExprError { at: start, kind: ExprErrorKind::UnknownUnit(other.into()) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mm(s: &str) -> f64 {
        eval_length_mm(s, false).unwrap()
    }

    #[test]
    fn numbers_units_and_arithmetic() {
        assert_eq!(mm("20"), 20.0);
        assert_eq!(mm(" 20 mm "), 20.0);
        assert_eq!(mm("1 in"), 25.4);
        assert_eq!(mm("2cm"), 20.0);
        assert_eq!(mm("25.4/2"), 12.7);
        assert_eq!(mm("(30 - 2*1.2)"), 27.6);
        assert_eq!(mm("-5 + 10"), 5.0);
        assert_eq!(mm(".5"), 0.5);
        assert_eq!(mm("1in + 0.6mm"), 26.0);
    }

    #[test]
    fn decimal_comma_only_when_enabled() {
        assert_eq!(eval_length_mm("12,5", true).unwrap(), 12.5);
        assert_eq!(eval_length_mm("12.5", true).unwrap(), 12.5);
        assert!(eval_length_mm("12,5", false).is_err());
    }

    #[test]
    fn errors_are_reported() {
        let kind = |s| eval_length_mm(s, false).unwrap_err().kind;
        assert_eq!(kind(""), ExprErrorKind::Empty);
        assert_eq!(kind("2 +"), ExprErrorKind::UnexpectedEnd);
        assert_eq!(kind("(2"), ExprErrorKind::UnexpectedEnd);
        assert_eq!(kind("2 ft"), ExprErrorKind::UnknownUnit("ft".into()));
        assert_eq!(kind("1/0"), ExprErrorKind::DivisionByZero);
        assert_eq!(kind("2 # 3"), ExprErrorKind::UnexpectedChar('#'));
    }
}
