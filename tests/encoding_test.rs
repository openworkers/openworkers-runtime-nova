mod common;

use common::js;
use common::js_err;

#[tokio::test]
async fn test_btoa_encodes_known_vectors() {
    for (input, expected) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
        ("\u{0}\u{1}\u{fe}\u{ff}", "AAH+/w=="),
    ] {
        assert_eq!(js(&format!("btoa('{input}')")).await, expected);
    }
}

#[tokio::test]
async fn test_atob_decodes_known_vectors() {
    for (input, expected) in [
        ("", ""),
        ("Zg==", "f"),
        ("Zm8=", "fo"),
        ("Zm9v", "foo"),
        ("Zm9vYmFy", "foobar"),
    ] {
        assert_eq!(js(&format!("atob('{input}')")).await, expected);
    }
}

#[tokio::test]
async fn test_atob_ignores_ascii_whitespace() {
    assert_eq!(js("atob(' Zm9v \\n YmFy ')").await, "foobar");
}

#[tokio::test]
async fn test_atob_accepts_unpadded_input() {
    assert_eq!(js("atob('Zm9vYmE')").await, "fooba");
}

#[tokio::test]
async fn test_atob_round_trips_every_byte() {
    let script = "atob(btoa(Array.from({ length: 256 }, (_, i) => \
                  String.fromCharCode(i)).join(''))).length";

    assert_eq!(js(script).await, "256");
}

#[tokio::test]
async fn test_btoa_rejects_a_code_point_above_ff() {
    let message = js_err("btoa('\\u0100')").await;

    assert!(message.contains("InvalidCharacterError"), "{message}");
}

#[tokio::test]
async fn test_atob_rejects_a_character_outside_the_alphabet() {
    let message = js_err("atob('Zm9v*')").await;

    assert!(message.contains("InvalidCharacterError"), "{message}");
}

#[tokio::test]
async fn test_atob_rejects_a_length_that_cannot_decode() {
    let message = js_err("atob('Zm9vY')").await;

    assert!(message.contains("InvalidCharacterError"), "{message}");
}

#[tokio::test]
async fn test_dom_exception_carries_its_name() {
    assert_eq!(
        js("new DOMException('m', 'DataError').name + ':' + \
            (new DOMException('m', 'DataError') instanceof Error)")
        .await,
        "DataError:true"
    );
}
