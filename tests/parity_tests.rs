use serde_json::{Value, json};
use tempfile::TempDir;
use wilysearch::{
    core::{Error, MeilisearchOptions},
    engine::Engine,
    traits::*,
    types::*,
};

fn setup() -> (Engine, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(MeilisearchOptions {
        db_path: dir.path().into(),
        ..Default::default()
    })
    .unwrap();
    (engine, dir)
}
fn index(engine: &Engine, uid: &str, settings: Value, docs: Value) {
    engine
        .create_index(&CreateIndexRequest {
            uid: uid.into(),
            primary_key: Some("id".into()),
        })
        .unwrap();
    engine
        .update_settings(uid, &serde_json::from_value(settings).unwrap())
        .unwrap();
    engine
        .add_or_replace_documents(uid, docs.as_array().unwrap(), &Default::default())
        .unwrap();
}
fn enable(engine: &Engine) {
    engine.update_experimental_features(&serde_json::from_value(json!({
        "foreignKeys":true,"dynamicSearchRules":true,"renderTemplates":true,"chatCompletions":true,
        "editDocumentsByFunction":true,"multimodal":true,"compositeEmbedders":true
    })).unwrap()).unwrap();
}
fn search(engine: &Engine, uid: &str, request: Value) -> SearchResponse {
    engine
        .search(uid, &serde_json::from_value(request).unwrap())
        .unwrap()
}

#[test]
fn federation_empty_json_and_rust_defaults() {
    let core: wilysearch::core::Federation = serde_json::from_value(json!({})).unwrap();
    assert_eq!(core.limit, 20);
    assert!(core.distinct.is_none());
    assert_eq!(
        serde_json::to_value(&core).unwrap(),
        serde_json::to_value(wilysearch::core::Federation::default()).unwrap()
    );
    let public: FederationSettings = serde_json::from_value(json!({})).unwrap();
    assert!(public.distinct.is_none());
    assert!(public.personalize.is_none());
    for federation in [
        json!({}),
        json!({"facetsByIndex":{"docs":["group"]},"mergeFacets":{}}),
    ] {
        let request: MultiSearchRequest =
            serde_json::from_value(json!({"queries":[],"federation":federation})).unwrap();
        assert!(request.federation.is_some());
    }
    let query: wilysearch::core::SearchQuery = serde_json::from_value(json!({})).unwrap();
    assert_eq!(
        serde_json::to_value(query).unwrap(),
        serde_json::to_value(wilysearch::core::SearchQuery::default()).unwrap()
    );
    let options: wilysearch::core::FederationOptions = serde_json::from_value(json!({})).unwrap();
    assert_eq!(options.weight, 1.0);
    assert_eq!(
        serde_json::to_value(options).unwrap(),
        serde_json::to_value(wilysearch::core::FederationOptions::default()).unwrap()
    );
}

#[test]
fn settings_reset_and_nested_documents() {
    let (engine, dir) = setup();
    index(
        &engine,
        "docs",
        json!({"filterableAttributes":["rank"],"sortableAttributes":["rank"],
        "displayedAttributes":["id","nested","tags"],"pagination":{"maxTotalHits":2},"prefixSearch":"disabled",
        "facetSearch":false,"searchCutoffMs":500,"typoTolerance":{"disableOnNumbers":true},
        "localizedAttributes":[{"attributePatterns":["nested.*"],"locales":["eng"]}]}),
        json!([{"id":1,"nested":{"title":"Rust search"},"secret":"hidden","tags":["Rust book","search"],"rank":2},
        {"id":2,"nested":{"title":"Go search"},"rank":1},{"id":3,"nested":{"title":"Other"},"rank":3}]),
    );
    let settings = engine.get_settings("docs").unwrap();
    engine.update_settings("docs", &settings).unwrap();
    drop(engine);
    let engine = Engine::new(MeilisearchOptions {
        db_path: dir.path().into(),
        ..Default::default()
    })
    .unwrap();
    let got = serde_json::to_value(engine.get_settings("docs").unwrap()).unwrap();
    assert_eq!(got, serde_json::to_value(settings).unwrap());
    assert_eq!(got["prefixSearch"], "disabled");
    assert_eq!(got["typoTolerance"]["disableOnNumbers"], true);
    let result = search(
        &engine,
        "docs",
        json!({"q":"Rust","attributesToRetrieve":["nested.title","tags","secret"],"attributesToHighlight":["nested.title","tags"],"showMatchesPosition":true}),
    );
    assert!(result.hits[0].get("secret").is_none());
    assert_eq!(
        result.hits[0]["_formatted"]["nested"]["title"],
        "<em>Rust</em> search"
    );
    assert_eq!(
        result.hits[0]["_formatted"]["tags"][0],
        "<em>Rust</em> book"
    );
    assert_eq!(
        result.hits[0]["_matchesPosition"]["tags"][0]["indices"],
        json!([0])
    );
    assert_eq!(search(&engine, "docs", json!({"limit":50})).hits.len(), 2);
    let fetched = engine.fetch_documents("docs",&serde_json::from_value(json!({"ids":[1,2],"filter":["rank >= 1",["rank = 2","rank = 3"]],"sort":["rank:desc"],"fields":["nested.title","secret"]})).unwrap()).unwrap();
    assert_eq!(fetched.total, 1);
    assert_eq!(fetched.results[0]["secret"], "hidden"); // browse ignores search visibility
    engine
        .add_or_update_documents(
            "docs",
            &[json!({"id":1,"rank":7}), json!({"id":9,"rank":9})],
            &AddDocumentsQuery {
                skip_creation: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(engine.index_stats("docs").unwrap().number_of_documents, 3);
    engine
        .update_settings(
            "docs",
            &serde_json::from_value(json!({"prefixSearch":null})).unwrap(),
        )
        .unwrap();
    let got = serde_json::to_value(engine.get_settings("docs").unwrap()).unwrap();
    assert_eq!(got["prefixSearch"], "indexingTime");
    assert_eq!(got["searchCutoffMs"], 500);
    assert!(
        engine
            .search(
                "docs",
                &serde_json::from_value(json!({"filter":[[false]]})).unwrap()
            )
            .is_err()
    );
    assert!(
        engine
            .search(
                "docs",
                &serde_json::from_value(json!({"page":1,"offset":0})).unwrap()
            )
            .is_err()
    );
    let fields = engine
        .list_fields(
            "docs",
            &serde_json::from_value(json!({"filter":{"sortable":true}})).unwrap(),
        )
        .unwrap();
    assert_eq!(fields.results[0]["name"], "rank");
}

#[test]
fn native_vectors_similar_and_federation() {
    let (engine, _dir) = setup();
    index(
        &engine,
        "docs",
        json!({"embedders":{"manual":{"source":"userProvided","dimensions":2}},"filterableAttributes":["group"],"faceting":{"sortFacetValuesBy":{"*":"count"}}}),
        json!([{"id":1,"title":"one","group":"z","_vectors":{"manual":[1.0,0.0]}},{"id":2,"title":"two","group":"z","_vectors":{"manual":[0.9,0.1]}},{"id":3,"title":"three","group":"b","_vectors":{"manual":[0.0,1.0]}}]),
    );
    let result = search(
        &engine,
        "docs",
        json!({"q":"no keyword matches","vector":[1,0],"hybrid":{"embedder":"manual","semanticRatio":1},"showRankingScore":true,"retrieveVectors":true}),
    );
    assert_eq!(result.hits[0]["id"], 1);
    assert_eq!(result.semantic_hit_count, Some(3));
    assert_eq!(
        result.hits[0]["_vectors"]["manual"]["embeddings"],
        json!([[1.0, 0.0]])
    );
    let similar = engine
        .similar(
            "docs",
            &serde_json::from_value(json!({"id":1,"embedder":"manual","retrieveVectors":true}))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(similar.hits[0]["id"], 2);
    assert!(similar.hits.iter().all(|h| h["id"] != 1));
    assert!(similar.hits[0].get("_vectors").is_some());
    let stats = engine.index_stats("docs").unwrap();
    assert_eq!(stats.number_of_embeddings, 3);
    assert_eq!(stats.number_of_embedded_documents, 3);
    let fed: MultiSearchRequest = serde_json::from_value(json!({"queries":[{"indexUid":"docs"},{"indexUid":"docs","federationOptions":{"weight":2}}],"federation":{"facetsByIndex":{"docs":["group"]},"mergeFacets":{}}})).unwrap();
    let MultiSearchResult::Federated(result) = engine.multi_search(&fed).unwrap() else {
        panic!()
    };
    assert_eq!(result.hits.len(), 3);
    assert_eq!(
        result.hits[0]["_federation"]["queriesPosition"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let facets = &result.facet_distribution.unwrap()["group"];
    assert_eq!(facets["z"], 2);
    assert_eq!(facets.first().unwrap().0, "z"); // count order differs from alphabetical order
    engine
        .update_settings(
            "docs",
            &serde_json::from_value(json!({"pagination":{"maxTotalHits":1}})).unwrap(),
        )
        .unwrap();
    let similar = engine
        .similar(
            "docs",
            &serde_json::from_value(json!({"id":1,"embedder":"manual","limit":50})).unwrap(),
        )
        .unwrap();
    assert_eq!(similar.hits.len(), 1);
    assert_eq!(similar.estimated_total_hits, 1);
    enable(&engine);
    let projected = search(
        &engine,
        "docs",
        json!({"attributesToRetrieve":["id"],"retrieveVectors":true}),
    );
    assert!(projected.hits[0].get("_vectors").is_some());
    assert!(projected.hits[0].get("title").is_none());
}

#[test]
fn settings_redact_credentials_but_backups_preserve_them() {
    let (engine, dir) = setup();
    index(
        &engine,
        "docs",
        json!({"embedders":{"default":{"source":"openAi","model":"text-embedding-3-small","dimensions":2,"url":"http://127.0.0.1:9/embeddings","apiKey":"mock-private-key"}}}),
        json!([]),
    );
    let settings = serde_json::to_value(engine.get_settings("docs").unwrap()).unwrap();
    assert_ne!(
        settings["embedders"]["default"]["apiKey"],
        "mock-private-key"
    );
    assert!(!settings.to_string().contains("mock-private-key"));
    let export = dir.path().join("export");
    engine
        .export(&serde_json::from_value(json!({"url":export})).unwrap())
        .unwrap();
    let settings: Value =
        serde_json::from_slice(&std::fs::read(export.join("docs/settings.json")).unwrap()).unwrap();
    assert_eq!(
        settings["embedders"]["default"]["apiKey"],
        "mock-private-key"
    );
    engine.create_dump().unwrap();
    let dump = std::fs::read_dir(dir.path().join("dumps"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let settings: Value =
        serde_json::from_slice(&std::fs::read(dump.join("docs/settings.json")).unwrap()).unwrap();
    assert_eq!(
        settings["embedders"]["default"]["apiKey"],
        "mock-private-key"
    );
}

#[test]
fn normalized_match_positions_use_original_bytes() {
    let (engine, _dir) = setup();
    index(
        &engine,
        "docs",
        json!({}),
        json!([{"id":1,"title":"Un café"}]),
    );
    let result = search(
        &engine,
        "docs",
        json!({"q":"cafe","attributesToHighlight":["title"],"showMatchesPosition":true}),
    );
    assert_eq!(result.hits[0]["_formatted"]["title"], "Un <em>café</em>");
    assert_eq!(result.hits[0]["_matchesPosition"]["title"][0]["start"], 3);
    assert_eq!(
        result.hits[0]["_matchesPosition"]["title"][0]["length"],
        "café".len()
    );
}

#[test]
fn keyword_only_hybrid_needs_no_embedder_and_default_embedder_is_public() {
    let (engine, _dir) = setup();
    index(&engine, "docs", json!({}), json!([{"id":1,"title":"Rust"}]));
    for hybrid in [
        json!({"semanticRatio":0}),
        json!({"semanticRatio":0,"embedder":"missing"}),
    ] {
        let result = search(&engine, "docs", json!({"q":"Rust","hybrid":hybrid}));
        assert_eq!(result.hits[0]["id"], 1);
        assert_eq!(result.semantic_hit_count, None);
    }
    let request: SearchRequest =
        serde_json::from_value(json!({"hybrid":{"semanticRatio":0.5}})).unwrap();
    let hybrid: HybridQuery = request.hybrid.unwrap();
    assert_eq!(hybrid.embedder, "default");
    assert_eq!(hybrid.semantic_ratio, 0.5);
    index(
        &engine,
        "vectors",
        json!({"embedders":{"default":{"source":"userProvided","dimensions":2}}}),
        json!([{"id":1,"_vectors":{"default":[1,0]}}]),
    );
    assert_eq!(
        search(
            &engine,
            "vectors",
            json!({"hybrid":{"semanticRatio":1},"vector":[1,0]})
        )
        .hits[0]["id"],
        1
    );
    for request in [
        json!({"hybrid":{"semanticRatio":-0.1}}),
        json!({"hybrid":{"semanticRatio":1.1}}),
        json!({"rankingScoreThreshold":2}),
        json!({"vector":[1,0]}),
    ] {
        assert!(matches!(
            engine.search("docs", &serde_json::from_value(request).unwrap()),
            Err(Error::InvalidSearchRequest(_))
        ));
    }
}

#[test]
fn rules_recover_after_environment_creation_and_protect_metadata() {
    let (engine, dir) = setup();
    enable(&engine);
    let reserved: RuleUid =
        serde_json::from_value(json!(milli::dynamic_search_rules::METADATA_UID)).unwrap();
    assert!(matches!(
        engine.update_search_rule(
            &reserved,
            serde_json::from_value(json!({"active":true})).unwrap()
        ),
        Err(Error::InvalidSearchRuleUid(_))
    ));
    assert!(matches!(
        engine.delete_search_rule(&reserved),
        Err(Error::InvalidSearchRuleUid(_))
    ));
    let path = dir.path().join("internal/rules");
    assert!(!path.exists());
    // Simulate failure immediately after opening LMDB, before configuring the rules index.
    std::fs::create_dir_all(&path).unwrap();
    let mut options = milli::heed::EnvOpenOptions::new().read_txn_without_tls();
    options.map_size(100 * 1024 * 1024);
    drop(milli::Index::new(options, &path, milli::CreateOrOpen::create_without_shards()).unwrap());
    assert!(path.join("data.mdb").exists());
    index(
        &engine,
        "docs",
        json!({}),
        json!([{"id":"local","title":"Batman Returns"},{"id":"remote","title":"Batman"}]),
    );
    let uid: RuleUid = serde_json::from_value(json!("pin-batman")).unwrap();
    engine.update_search_rule(&uid, serde_json::from_value(json!({"active":true,"conditions":{"query":{"words":"returns"}},"actions":{"pin":[{"id":"remote","position":0}]}})).unwrap()).unwrap();
    assert!(matches!(
        engine.update_search_rule(
            &reserved,
            serde_json::from_value(json!({"active":false})).unwrap()
        ),
        Err(Error::InvalidSearchRuleUid(_))
    ));
    assert!(matches!(
        engine.delete_search_rule(&reserved),
        Err(Error::InvalidSearchRuleUid(_))
    ));
    assert!(engine.get_search_rule(&reserved).unwrap().is_none());
    assert_eq!(engine.list_search_rules(0, 10).unwrap().len(), 1);
    assert_eq!(
        search(&engine, "docs", json!({"q":"Batman Returns"})).hits[0]["id"],
        "remote"
    );
    drop(engine);
    let engine = Engine::new(MeilisearchOptions {
        db_path: dir.path().into(),
        ..Default::default()
    })
    .unwrap();
    enable(&engine);
    assert_eq!(
        search(&engine, "docs", json!({"q":"Batman Returns"})).hits[0]["id"],
        "remote"
    );
    assert!(engine.delete_search_rule(&uid).unwrap());
    assert!(engine.list_search_rules(0, 10).unwrap().is_empty());
}

#[test]
fn foreign_filters_hydration_and_feature_gate() {
    let (engine, _dir) = setup();
    enable(&engine);
    index(
        &engine,
        "authors",
        json!({"filterableAttributes":["name"],"displayedAttributes":["name"]}),
        json!([{"id":"a","name":"Ada","secret":true},{"id":"b","name":"Grace"}]),
    );
    index(
        &engine,
        "books",
        json!({"filterableAttributes":["author","genre"],"foreignKeys":[{"fieldName":"author","foreignIndexUid":"authors"}],"embedders":{"manual":{"source":"userProvided","dimensions":2}}}),
        json!([{"id":1,"title":"Compiler","genre":"code","author":"a","_vectors":{"manual":[1.0,0.0]}},{"id":2,"title":"Code","genre":"code","author":["b","missing"],"_vectors":{"manual":[0.9,0.1]}}]),
    );
    let result = search(
        &engine,
        "books",
        json!({"filter":"_foreign(author, name = Ada)","attributesToRetrieve":["id","author.name"]}),
    );
    assert_eq!(
        result.hits,
        json!([{"id":1,"author":{"name":"Ada"}}])
            .as_array()
            .unwrap()
            .clone()
    );
    let result = search(&engine, "books", json!({}));
    assert_eq!(result.hits[1]["author"][1], json!({}));
    let similar = engine.similar("books", &serde_json::from_value(json!({"id":2,"embedder":"manual","attributesToRetrieve":["author.name"],"retrieveVectors":true})).unwrap()).unwrap();
    assert_eq!(similar.hits[0]["author"], json!({"name":"Ada"}));
    assert!(similar.hits[0].get("id").is_none());
    assert!(similar.hits[0].get("_vectors").is_some());
    let facet = engine
        .facet_search(
            "books",
            &serde_json::from_value(
                json!({"facetName":"genre","filter":"_foreign(author, name = Ada)"}),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(facet.facet_hits[0].count, 1);
    engine
        .update_experimental_features(&Default::default())
        .unwrap();
    assert!(matches!(
        engine.search(
            "books",
            &serde_json::from_value(json!({"filter":"_foreign(author, name = Ada)"})).unwrap()
        ),
        Err(Error::ExperimentalFeatureNotEnabled(_))
    ));
}

#[test]
fn rules_templates_edits_rename_and_snapshot() {
    let (engine, dir) = setup();
    enable(&engine);
    index(
        &engine,
        "docs",
        json!({"filterableAttributes":["id"],"chat":{"description":"Examples","documentTemplate":"{{ doc.title }}"}}),
        json!([{"id":"local","title":"Batman Returns"},{"id":"remote","title":"Batman"}]),
    );
    let uid: RuleUid = serde_json::from_value(json!("pin-batman")).unwrap();
    engine.update_search_rule(&uid,serde_json::from_value(json!({"active":true,"conditions":{"query":{"words":"returns"}},"actions":{"pin":[{"id":"remote","position":0}]}})).unwrap()).unwrap();
    assert_eq!(
        search(&engine, "docs", json!({"q":"Batman Returns"})).hits[0]["id"],
        "remote"
    );
    assert_eq!(engine.list_search_rules(0, 10).unwrap().len(), 1);
    let request: RenderRequest = serde_json::from_value(json!({"template":{"kind":"chatDocumentTemplate","indexUid":"docs"},"input":{"kind":"indexDocument","indexUid":"docs","id":"local"}})).unwrap();
    assert_eq!(
        engine.render_template(&request).unwrap().rendered,
        Some(json!("Batman Returns"))
    );
    let fragment: RenderRequest = serde_json::from_value(json!({"template":{"kind":"inlineFragment","inline":{"text":"{{ q }}"}},"input":{"kind":"inlineSearch","inline":{"q":"hello"}}})).unwrap();
    assert_eq!(
        engine.render_template(&fragment).unwrap().rendered,
        Some(json!({"text":"hello"}))
    );
    engine.update_documents_by_function("docs",&serde_json::from_value(json!({"function":"doc.title = context.title","filter":"id = local","context":{"title":"Changed"}})).unwrap()).unwrap();
    assert_eq!(
        engine
            .get_document("docs", "local", &Default::default())
            .unwrap()["title"],
        "Changed"
    );
    let bad = serde_json::from_value(json!({"function":"doc.id = \"changed\""})).unwrap();
    assert!(engine.update_documents_by_function("docs", &bad).is_err());
    assert_eq!(engine.index_stats("docs").unwrap().number_of_documents, 2);
    engine.rename_index("docs", "renamed").unwrap();
    engine.create_snapshot().unwrap();
    drop(engine);
    let restored = Engine::new(MeilisearchOptions {
        db_path: dir.path().join("snapshots"),
        ..Default::default()
    })
    .unwrap();
    enable(&restored);
    assert_eq!(restored.list_search_rules(0, 10).unwrap().len(), 1);
    assert_eq!(
        restored
            .get_document("renamed", "local", &Default::default())
            .unwrap()["title"],
        "Changed"
    );
    assert!(
        restored
            .list_indexes(&Default::default())
            .unwrap()
            .results
            .iter()
            .all(|i| i.uid != "rules")
    );
}

#[test]
fn legacy_storage_is_rejected_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("indexes/legacy");
    std::fs::create_dir_all(&index).unwrap();
    std::fs::write(index.join("data.mdb"), b"legacy").unwrap();
    assert!(matches!(
        Engine::new(MeilisearchOptions {
            db_path: dir.path().into(),
            ..Default::default()
        }),
        Err(Error::IncompatibleDatabase(_))
    ));
    assert_eq!(std::fs::read(index.join("data.mdb")).unwrap(), b"legacy");
    assert!(!dir.path().join("wilysearch-format.json").exists());
    assert!(!index.join("metadata.json").exists());
}

#[test]
fn federation_sort_compatibility_and_rule_scaling() {
    let (engine, _dir) = setup();
    enable(&engine);
    index(
        &engine,
        "a",
        json!({"filterableAttributes":["id"],"sortableAttributes":["price"]}),
        json!([{"id":1,"title":"doc","price":5},{"id":2,"title":"doc","price":1}]),
    );
    index(
        &engine,
        "b",
        json!({"sortableAttributes":["price"]}),
        json!([{"id":1,"title":"doc","price":3}]),
    );
    let request: MultiSearchRequest = serde_json::from_value(json!({"queries":[{"indexUid":"a","sort":["price:asc"]},{"indexUid":"b","sort":["price:asc"]}],"federation":{}})).unwrap();
    let MultiSearchResult::Federated(result) = engine.multi_search(&request).unwrap() else {
        panic!()
    };
    assert_eq!(
        result
            .hits
            .iter()
            .map(|h| h["price"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 3, 5]
    );
    let mut bad = request.clone();
    bad.queries[1].search.sort = Some(vec!["price:desc".into()]);
    assert!(engine.multi_search(&bad).is_err());
    let uid = serde_json::from_value(json!("hide-one")).unwrap();
    engine.update_search_rule(&uid,serde_json::from_value(json!({"active":true,"conditions":{"query":{"words":"doc"}},"actions":{"scale":[{"ids":["1"],"weight":0.0}]}})).unwrap()).unwrap();
    let result = search(&engine, "a", json!({"q":"doc"}));
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0]["id"], 2);
    assert!(search(&engine, "a", json!({"page":0})).hits.is_empty());
    engine
        .delete_documents_by_filter(
            "a",
            &serde_json::from_value(json!({"filter":[["id = 1","id = 2"]]})).unwrap(),
        )
        .unwrap();
    assert_eq!(engine.index_stats("a").unwrap().number_of_documents, 0);
}
