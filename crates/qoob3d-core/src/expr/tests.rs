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
