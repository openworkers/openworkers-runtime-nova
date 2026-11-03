mod common;

use common::js;

/// Reads a stream to the end as text.
const READ: &str = "const read = (stream) => new Response(stream).text();";

#[tokio::test]
async fn test_each_format_round_trips() {
    let script = format!(
        r#"
        (async () => {{
            {READ}
            const out = [];

            for (const format of ['gzip', 'deflate', 'deflate-raw']) {{
                const text = 'Pack my box with five dozen liquor jugs. '.repeat(50);
                const packed = new Blob([text]).stream().pipeThrough(new CompressionStream(format));
                const back = packed.pipeThrough(new DecompressionStream(format));

                out.push((await read(back)) === text);
            }}

            return out.join();
        }})()
    "#
    );

    assert_eq!(js(&script).await, "true,true,true");
}

#[tokio::test]
async fn test_gzip_output_has_the_gzip_magic_and_shrinks_repeated_text() {
    let script = r#"
        (async () => {
            const text = 'a'.repeat(10000);
            const packed = new Uint8Array(
                await new Response(new Blob([text]).stream().pipeThrough(new CompressionStream('gzip'))).arrayBuffer()
            );

            return [packed[0].toString(16), packed[1].toString(16), packed.length < 100].join();
        })()
    "#;

    assert_eq!(js(script).await, "1f,8b,true");
}

#[tokio::test]
async fn test_a_stream_made_elsewhere_decompresses() {
    // `hello` through zlib, from outside this runtime.
    let script = format!(
        r#"
        (async () => {{
            {READ}
            const zlib = new Uint8Array([0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00, 0x06, 0x2c, 0x02, 0x15]);

            return await read(new Blob([zlib]).stream().pipeThrough(new DecompressionStream('deflate')));
        }})()
    "#
    );

    assert_eq!(js(&script).await, "hello");
}

/// Bytes, not text: the JavaScript UTF-8 codec costs far more than the codecs
/// under test, and would be what this measures.
#[tokio::test]
async fn test_many_chunks_and_a_large_body_round_trip() {
    let script = r#"
        (async () => {
            const chunk = crypto.getRandomValues(new Uint8Array(65536));
            const source = new ReadableStream({
                start(controller) {
                    for (let i = 0; i < 16; i++) {
                        controller.enqueue(chunk.slice());
                    }

                    controller.close();
                },
            });

            const back = new Uint8Array(
                await new Response(
                    source.pipeThrough(new CompressionStream('gzip')).pipeThrough(new DecompressionStream('gzip'))
                ).arrayBuffer()
            );

            const same = back.length === chunk.length * 16 && [0, 1, 65535, 65536, 1048575].every((at) => back[at] === chunk[at % 65536]);

            return same;
        })()
    "#;

    assert_eq!(js(script).await, "true");
}

#[tokio::test]
async fn test_a_format_the_standard_does_not_name_is_refused() {
    let script = r#"
        (() => {
            const out = [];

            for (const make of [() => new CompressionStream('brotli'), () => new DecompressionStream('zstd')]) {
                try {
                    make();
                    out.push('accepted');
                } catch (error) {
                    out.push(error.name);
                }
            }

            return out.join();
        })()
    "#;

    assert_eq!(js(script).await, "TypeError,TypeError");
}

#[tokio::test]
async fn test_data_that_is_not_compressed_rejects_the_read() {
    let script = format!(
        r#"
        (async () => {{
            {READ}

            try {{
                await read(new Blob([new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8])]).stream().pipeThrough(new DecompressionStream('gzip')));

                return 'accepted';
            }} catch (error) {{
                return error.name;
            }}
        }})()
    "#
    );

    assert_eq!(js(&script).await, "TypeError");
}
