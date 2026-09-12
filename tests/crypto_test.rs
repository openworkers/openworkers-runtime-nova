mod common;

use common::js;
use common::js_err;

#[tokio::test]
async fn test_get_random_values_fills_the_view_and_returns_it() {
    let script = r#"
        (() => {
            const view = new Uint8Array(32);
            const same = crypto.getRandomValues(view) === view;
            const untouched = view.every((byte) => byte === 0);

            return same + ':' + untouched;
        })()
    "#;

    assert_eq!(js(script).await, "true:false");
}

#[tokio::test]
async fn test_two_draws_differ() {
    let script = "crypto.getRandomValues(new Uint8Array(16)).join(',') === \
                  crypto.getRandomValues(new Uint8Array(16)).join(',')";

    assert_eq!(js(script).await, "false");
}

#[tokio::test]
async fn test_wider_integer_views_are_filled() {
    let script = "(() => { const v = new Uint32Array(4); crypto.getRandomValues(v); \
                  return v.every((word) => word === 0); })()";

    assert_eq!(js(script).await, "false");
}

#[tokio::test]
async fn test_only_a_part_of_the_buffer_is_written() {
    let script = r#"
        (() => {
            const buffer = new Uint8Array(8);

            crypto.getRandomValues(buffer.subarray(2, 6));

            return [buffer[0], buffer[1], buffer[6], buffer[7]].join(',');
        })()
    "#;

    assert_eq!(js(script).await, "0,0,0,0");
}

#[tokio::test]
async fn test_a_view_that_is_not_an_integer_array_is_rejected() {
    for expression in [
        "crypto.getRandomValues(new Float64Array(2))",
        "crypto.getRandomValues([1, 2])",
        "crypto.getRandomValues(new DataView(new ArrayBuffer(4)))",
    ] {
        assert!(
            js_err(expression).await.contains("TypeMismatchError"),
            "{expression}"
        );
    }
}

#[tokio::test]
async fn test_more_than_the_quota_is_refused() {
    let message = js_err("crypto.getRandomValues(new Uint8Array(65537))").await;

    assert!(message.contains("QuotaExceededError"), "{message}");
}

#[tokio::test]
async fn test_random_uuid_has_the_version_four_shape() {
    let script = "crypto.randomUUID()";
    let uuid = js(script).await;
    let parts: Vec<&str> = uuid.split('-').collect();

    assert_eq!(parts.len(), 5, "{uuid}");
    assert_eq!(
        parts.iter().map(|part| part.len()).collect::<Vec<_>>(),
        vec![8, 4, 4, 4, 12],
        "{uuid}"
    );
    assert!(
        uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "{uuid}"
    );
    assert!(parts[2].starts_with('4'), "{uuid}");
    assert!(
        ['8', '9', 'a', 'b'].contains(&parts[3].chars().next().expect("variant nibble")),
        "{uuid}"
    );

    assert_ne!(js(script).await, uuid);
}

#[tokio::test]
async fn test_an_aes_gcm_message_that_was_tampered_with_is_refused() {
    let script = r#"
        (async () => {
            const key = await crypto.subtle.generateKey(
                { name: 'AES-GCM', length: 256 }, true, ['encrypt', 'decrypt']
            );
            const iv = crypto.getRandomValues(new Uint8Array(12));
            const cipher = new Uint8Array(
                await crypto.subtle.encrypt({ name: 'AES-GCM', iv: iv }, key, new Uint8Array([1, 2, 3]))
            );

            cipher[0] ^= 1;

            try {
                await crypto.subtle.decrypt({ name: 'AES-GCM', iv: iv }, key, cipher);

                return 'decrypted anyway';
            } catch (error) {
                return error.name;
            }
        })()
    "#;

    assert_eq!(js(script).await, "OperationError");
}

#[tokio::test]
async fn test_a_key_imported_as_not_extractable_cannot_be_exported() {
    let script = r#"
        (async () => {
            const key = await crypto.subtle.importKey(
                'raw', new Uint8Array([1, 2, 3]), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']
            );

            try {
                await crypto.subtle.exportKey('raw', key);

                return 'exported anyway';
            } catch (error) {
                return error.name;
            }
        })()
    "#;

    assert_eq!(js(script).await, "InvalidAccessError");
}
