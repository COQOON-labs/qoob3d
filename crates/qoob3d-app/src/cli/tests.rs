use super::*;

#[test]
fn size_parsing() {
    assert_eq!(parse_size("1280x800"), Ok([1280, 800]));
    assert_eq!(parse_size("64X64"), Ok([64, 64]));
    assert!(parse_size("1280").is_err());
    assert!(parse_size("0x10").is_err());
    assert!(parse_size("99999x10").is_err());
}

#[test]
fn cli_definition_is_valid() {
    use clap::CommandFactory;
    Cli::command().debug_assert();
}
