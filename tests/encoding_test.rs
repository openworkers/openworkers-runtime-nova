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

#[tokio::test]
async fn test_text_encoder_encodes_utf8() {
    for (input, expected) in [
        ("", ""),
        ("abc", "97,98,99"),
        ("\u{e9}", "195,169"),
        ("\u{20ac}", "226,130,172"),
        ("\u{1f600}", "240,159,152,128"),
    ] {
        let script = format!("new TextEncoder().encode('{input}').join(',')");

        assert_eq!(js(&script).await, expected);
    }
}

#[tokio::test]
async fn test_text_encoder_replaces_a_lone_surrogate() {
    assert_eq!(
        js("new TextEncoder().encode('\\ud800').join(',')").await,
        "239,191,189"
    );
}

#[tokio::test]
async fn test_text_encoder_returns_a_uint8array() {
    let script = "(new TextEncoder().encode('a') instanceof Uint8Array) + ':' + \
                  new TextEncoder().encoding";

    assert_eq!(js(script).await, "true:utf-8");
}

#[tokio::test]
async fn test_text_decoder_decodes_utf8() {
    let script = "new TextDecoder().decode(new Uint8Array([240,159,152,128,226,130,172,97]))";

    assert_eq!(js(script).await, "\u{1f600}\u{20ac}a");
}

#[tokio::test]
async fn test_text_decoder_round_trips_the_encoder() {
    let script = "new TextDecoder().decode(new TextEncoder().encode('h\u{e9} \u{1f600} \u{4e16}'))";

    assert_eq!(js(script).await, "h\u{e9} \u{1f600} \u{4e16}");
}

#[tokio::test]
async fn test_text_decoder_replaces_malformed_sequences() {
    for (bytes, expected) in [
        ("[0xff]", "\u{fffd}"),
        ("[0xc3]", "\u{fffd}"),
        ("[0xc3,0x28]", "\u{fffd}("),
        ("[0xed,0xa0,0x80]", "\u{fffd}\u{fffd}\u{fffd}"),
        ("[0xf5,0x80,0x80,0x80]", "\u{fffd}\u{fffd}\u{fffd}\u{fffd}"),
        ("[0x61,0xe2,0x82,0x61]", "a\u{fffd}a"),
    ] {
        let script = format!("new TextDecoder().decode(new Uint8Array({bytes}))");

        assert_eq!(js(&script).await, expected, "for bytes {bytes}");
    }
}

#[tokio::test]
async fn test_a_fatal_decoder_throws_on_malformed_input() {
    let script = "new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array([0xff]))";

    assert!(js_err(script).await.contains("TypeError"));
}

#[tokio::test]
async fn test_text_decoder_joins_streamed_chunks() {
    let script = r#"
        (() => {
            const decoder = new TextDecoder();
            const head = decoder.decode(new Uint8Array([0xf0, 0x9f]), { stream: true });
            const tail = decoder.decode(new Uint8Array([0x98, 0x80]), { stream: true });

            return JSON.stringify([head, tail]);
        })()
    "#;

    assert_eq!(js(script).await, "[\"\",\"\u{1f600}\"]");
}

#[tokio::test]
async fn test_text_decoder_strips_a_leading_bom() {
    let bom = "[0xef,0xbb,0xbf,0x61]";
    let script = format!("new TextDecoder().decode(new Uint8Array({bom}))");
    let kept =
        format!("new TextDecoder('utf-8', {{ ignoreBOM: true }}).decode(new Uint8Array({bom}))");

    assert_eq!(js(&script).await, "a");
    assert_eq!(js(&kept).await, "\u{feff}a");
}

#[tokio::test]
async fn test_text_decoder_accepts_buffers_and_views() {
    let script = "new TextDecoder().decode(new Uint8Array([97,98,99]).buffer) + ':' + \
                  new TextDecoder().decode(new Uint8Array([97,98,99,100]).subarray(1, 3)) + ':' + \
                  new TextDecoder().decode()";

    assert_eq!(js(script).await, "abc:bc:");
}

#[tokio::test]
async fn test_text_decoder_rejects_an_encoding_it_cannot_decode() {
    assert!(
        js_err("new TextDecoder('latin1')")
            .await
            .contains("RangeError")
    );
    assert_eq!(js("new TextDecoder('UTF8').encoding").await, "utf-8");
}
