#![cfg(feature = "ai")]
use futures::StreamExt;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use wilysearch::{
    ai::{Chat, ChatEvent, CohereConfig, CohereReranker, WorkspaceSettings},
    core::MeilisearchOptions,
    engine::Engine,
    traits::*,
    types::*,
};

fn read_request(stream: &mut TcpStream) -> (String, Value) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut bytes = Vec::new();
    let split = loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break bytes.len();
        }
    };
    let headers = String::from_utf8(bytes).unwrap();
    let length: usize = headers
        .lines()
        .find_map(|line| {
            line.to_lowercase()
                .strip_prefix("content-length:")
                .map(|s| s.trim().parse().unwrap())
        })
        .unwrap();
    assert!(split < 100_000 && length < 1_000_000);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    (headers, serde_json::from_slice(&body).unwrap())
}
fn server(
    responses: Vec<(u16, Value)>,
) -> (
    String,
    mpsc::Receiver<(String, Value)>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let join = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        for (status, response) in responses {
            let start = Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            start.elapsed() < Duration::from_secs(15),
                            "provider was not called"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            tx.send(read_request(&mut stream)).unwrap();
            let body = response.to_string();
            write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    (url, rx, join)
}
fn completion(message: Value) -> Value {
    json!({"id":"test","object":"chat.completion","created":0,"model":"mock","choices":[{"index":0,"message":message,"finish_reason":"stop"}]})
}
fn engine() -> (Arc<Engine>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Arc::new(
        Engine::new(MeilisearchOptions {
            db_path: dir.path().into(),
            allow_local_provider_urls: true,
            ..Default::default()
        })
        .unwrap(),
    );
    engine
        .update_experimental_features(
            &serde_json::from_value(json!({"chatCompletions":true})).unwrap(),
        )
        .unwrap();
    engine
        .create_index(&CreateIndexRequest {
            uid: "docs".into(),
            primary_key: Some("id".into()),
        })
        .unwrap();
    engine.update_settings("docs",&serde_json::from_value(json!({"chat":{"documentTemplate":"Title: {{ doc.title }}","searchParameters":{"limit":1}}})).unwrap()).unwrap();
    engine
        .add_or_replace_documents(
            "docs",
            &[json!({"id":1,"title":"Rust"})],
            &Default::default(),
        )
        .unwrap();
    (engine, dir)
}
fn request() -> wilysearch::ai::CreateChatCompletionRequest {
    serde_json::from_value(json!({"model":"mock","messages":[{"role":"user","content":"Rust?"}]}))
        .unwrap()
}

#[tokio::test]
async fn chat_tools_workspace_redaction_and_caller_tools() {
    let (url, requests, join) = server(vec![
        (
            200,
            completion(
                json!({"role":"assistant","content":null,"tool_calls":[{"id":"call1","type":"function","function":{"name":"_meiliSearchInIndex","arguments":"{\"index_uid\":\"docs\",\"q\":\"Rust\",\"filter\":\"\"}"}}]}),
            ),
        ),
        (
            200,
            completion(
                json!({"role":"assistant","content":"Found Rust","tool_calls":[{"id":"custom","type":"function","function":{"name":"save","arguments":"{}"}}]}),
            ),
        ),
    ]);
    let (engine, dir) = engine();
    engine
        .set_chat_workspace(
            "default",
            serde_json::from_value(json!({"source":"vLlm","baseUrl":url,"apiKey":"mock-key"}))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        engine
            .get_chat_workspace("default")
            .unwrap()
            .unwrap()
            .api_key
            .as_deref(),
        Some("[redacted]")
    );
    let mut req = request();
    req.tools = Some(
        serde_json::from_value(
            json!([{"type":"function","function":{"name":"save","parameters":{"type":"object"}}}]),
        )
        .unwrap(),
    );
    let result = Chat::new(engine.clone(), "default")
        .complete(req)
        .await
        .unwrap();
    assert_eq!(
        result.response.choices[0].message.content.as_deref(),
        Some("Found Rust")
    );
    assert_eq!(result.sources[0].results.hits[0]["title"], "Rust");
    assert_eq!(
        result.response.choices[0]
            .message
            .tool_calls
            .as_ref()
            .unwrap()[0]
            .function
            .name,
        "save"
    );
    let (_, first) = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(first["messages"][0]["role"], "system");
    let (_, second) = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(
        second["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool" && m["content"].as_str().unwrap().contains("Title: Rust"))
    );
    join.join().unwrap();
    engine.create_snapshot().unwrap();
    drop(engine);
    let restored = Engine::new(MeilisearchOptions {
        db_path: dir.path().join("snapshots"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(restored.list_chat_workspaces().unwrap(), vec!["default"]);
}

#[tokio::test]
async fn stream_delivers_deltas_before_provider_finishes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (release, wait) = mpsc::channel();
    let join = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let (_, request) = read_request(&mut stream);
        assert_eq!(request["stream"], true);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let chunk = |text| {
            json!({"id":"stream","object":"chat.completion.chunk","created":0,"model":"mock","choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]}).to_string()
        };
        write!(stream, "data: {}\n\n", chunk("First")).unwrap();
        stream.flush().unwrap();
        wait.recv_timeout(Duration::from_secs(5))
            .expect("first delta was buffered");
        write!(stream, "data: {}\n\ndata: [DONE]\n\n", chunk(" second")).unwrap();
    });
    let (engine, _dir) = engine();
    engine
        .set_chat_workspace(
            "stream",
            serde_json::from_value(json!({"source":"mistral","baseUrl":url,"apiKey":"mock-key"}))
                .unwrap(),
        )
        .unwrap();
    let mut stream = Chat::new(engine, "stream").stream(request());
    let first = tokio::time::timeout(Duration::from_secs(4), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ChatEvent::Chunk { chunk } = first else {
        panic!("expected live chunk")
    };
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("First"));
    release.send(()).unwrap();
    let mut done = false;
    while let Some(event) = stream.next().await {
        if let ChatEvent::Done { messages } = event.unwrap() {
            assert_eq!(
                serde_json::to_value(messages.last().unwrap()).unwrap()["content"],
                "First second"
            );
            done = true;
        }
    }
    assert!(done);
    join.join().unwrap();
}

#[tokio::test]
async fn azure_routing_and_validation() {
    let (url, requests, join) = server(vec![(
        200,
        completion(json!({"role":"assistant","content":"ok"})),
    )]);
    let (engine, _dir) = engine();
    let settings: WorkspaceSettings = serde_json::from_value(json!({"source":"azureOpenAi","baseUrl":url,"apiKey":"mock-key","apiVersion":"2025-01-01","deploymentId":"test"})).unwrap();
    engine.set_chat_workspace("azure", settings).unwrap();
    Chat::new(engine.clone(), "azure")
        .complete(request())
        .await
        .unwrap();
    let (headers, _) = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(
        headers
            .starts_with("POST /openai/deployments/test/chat/completions?api-version=2025-01-01")
    );
    assert!(headers.to_lowercase().contains("api-key: mock-key"));
    join.join().unwrap();
    assert!(
        engine
            .set_chat_workspace(
                "bad",
                serde_json::from_value(json!({"source":"vLlm"})).unwrap()
            )
            .is_err()
    );
    let mut invalid = request();
    invalid.n = Some(2);
    assert!(Chat::new(engine, "azure").complete(invalid).await.is_err());
}

#[test]
fn cohere_retries_order_deadline_and_invalid_response() {
    let (url, requests, join) = server(vec![
        (429, json!({"error":"retry"})),
        (200, json!({"results":[{"index":1},{"index":0}]})),
    ]);
    let reranker = CohereReranker::new(
        CohereConfig {
            api_key: "mock-key".into(),
            url,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    let docs = vec![json!({"id":1}), json!({"id":2})];
    assert_eq!(
        reranker
            .order(
                Some("query"),
                "context",
                &docs,
                Instant::now() + Duration::from_secs(3)
            )
            .unwrap(),
        vec![1, 0]
    );
    let (_, request) = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(request["query"], "User Context: context\nQuery: query");
    assert_eq!(
        reranker.order(None, "", &docs, Instant::now()).unwrap(),
        vec![0, 1]
    );
    join.join().unwrap();
    let (url, _requests, join) = server(vec![(200, json!({"results":[{"index":0},{"index":0}]}))]);
    let reranker = CohereReranker::new(
        CohereConfig {
            api_key: "mock-key".into(),
            url,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    assert!(
        reranker
            .order(None, "", &docs, Instant::now() + Duration::from_secs(2))
            .is_err()
    );
    join.join().unwrap();
}

#[test]
fn personalization_preserves_pins_and_scores() {
    let (url, requests, join) = server(vec![(200, json!({"results":[{"index":1},{"index":0}]}))]);
    let (engine, _dir) = engine();
    engine
        .update_experimental_features(
            &serde_json::from_value(json!({"dynamicSearchRules":true})).unwrap(),
        )
        .unwrap();
    engine
        .add_or_replace_documents(
            "docs",
            &[
                json!({"id":2,"title":"Rust two"}),
                json!({"id":3,"title":"Rust three"}),
            ],
            &Default::default(),
        )
        .unwrap();
    let uid = serde_json::from_value(json!("pin-one")).unwrap();
    engine
        .update_search_rule(
            &uid,
            serde_json::from_value(
                json!({"active":true,"actions":{"pin":[{"id":"1","position":0}]}}),
            )
            .unwrap(),
        )
        .unwrap();
    engine
        .set_personalization(Some(CohereConfig {
            api_key: "mock-key".into(),
            url,
            ..Default::default()
        }))
        .unwrap();
    let baseline = engine
        .search(
            "docs",
            &serde_json::from_value(json!({"q":"Rust","showRankingScore":true})).unwrap(),
        )
        .unwrap();
    let result=engine.search("docs",&serde_json::from_value(json!({"q":"Rust","showRankingScore":true,"personalize":{"userContext":"prefer three"}})).unwrap()).unwrap();
    assert_eq!(result.hits[0], baseline.hits[0]);
    assert_eq!(result.hits[1], baseline.hits[2]);
    assert_eq!(result.hits[2], baseline.hits[1]);
    let (_, body) = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(body["documents"].as_array().unwrap().len(), 2);
    join.join().unwrap();
}

#[tokio::test]
async fn streaming_search_tool_round_trip() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let join = std::thread::spawn(move || {
        for round in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let (_, request) = read_request(&mut stream);
            if round == 1 {
                assert!(
                    request["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|m| m["role"] == "tool")
                );
            }
            let delta = if round == 0 {
                json!({"tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":"_meiliSearchInIndex","arguments":"{\"index_uid\":\"docs\",\"q\":\"Rust\",\"filter\":\"\"}"}}]})
            } else {
                json!({"content":"Found it"})
            };
            let chunk = json!({"id":"stream","object":"chat.completion.chunk","created":0,"model":"mock","choices":[{"index":0,"delta":delta,"finish_reason":if round==0 {"tool_calls"}else{"stop"}}]});
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {chunk}\n\ndata: [DONE]\n\n").unwrap();
        }
    });
    let (engine, _dir) = engine();
    engine
        .set_chat_workspace(
            "stream",
            serde_json::from_value(json!({"source":"vLlm","baseUrl":url})).unwrap(),
        )
        .unwrap();
    let mut stream = Chat::new(engine, "stream").stream(request());
    let mut sources = 0;
    let mut tools = 0;
    let mut done = false;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            ChatEvent::Sources { source } => {
                sources += 1;
                assert_eq!(source.results.hits[0]["title"], "Rust");
            }
            ChatEvent::ToolResult { .. } => tools += 1,
            ChatEvent::Done { .. } => done = true,
            _ => {}
        }
    }
    assert_eq!((sources, tools, done), (1, 1, true));
    join.join().unwrap();
}
