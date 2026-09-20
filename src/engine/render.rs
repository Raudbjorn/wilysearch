use super::Engine;
use crate::{core::Error, traits::Result, types::*};
use milli::{prompt::Prompt, vector::json_template::JsonTemplate};
use serde_json::Value;
use std::{cell::RefCell, sync::RwLock};

impl Engine {
    pub fn render_template(&self, request: &RenderRequest) -> Result<RenderResponse> {
        use RenderTemplate::*;
        let flags = self.inner.get_experimental_features();
        if !flags.render_templates {
            return Err(Error::ExperimentalFeatureNotEnabled(
                "renderTemplates".into(),
            ));
        }
        let fragment = matches!(
            request.template,
            InlineFragment { .. } | IndexingFragment { .. } | SearchFragment { .. }
        );
        if fragment && !flags.multimodal {
            return Err(Error::ExperimentalFeatureNotEnabled("multimodal".into()));
        }
        let (template, max_bytes) = match &request.template {
            InlineDocumentTemplate {
                inline,
                document_template_max_bytes,
            } => (Value::String(inline.clone()), *document_template_max_bytes),
            InlineFragment { inline } => (inline.clone(), None),
            ChatDocumentTemplate {
                index_uid,
                document_template_max_bytes,
            } => {
                if !flags.chat_completions {
                    return Err(Error::ExperimentalFeatureNotEnabled(
                        "chatCompletions".into(),
                    ));
                }
                let index = self.resolve_index(index_uid)?;
                let txn = index.inner.read_txn()?;
                let prompt = index.inner.chat_config(&txn)?.prompt;
                (
                    Value::String(prompt.template),
                    document_template_max_bytes.or(prompt.max_bytes),
                )
            }
            DocumentTemplate {
                index_uid,
                embedder,
                document_template_max_bytes,
            } => {
                let index = self.resolve_index(index_uid)?;
                let txn = index.inner.read_txn()?;
                let config = index
                    .inner
                    .embedding_configs()
                    .embedding_configs(&txn)?
                    .into_iter()
                    .find(|c| &c.name == embedder)
                    .ok_or_else(|| Error::EmbedderNotFound(embedder.clone()))?;
                if !config.config.embedder_options.has_document_template() {
                    return Err(Error::Internal(
                        "Embedder does not use a document template".into(),
                    ));
                }
                (
                    Value::String(config.config.prompt.template),
                    document_template_max_bytes.or(config.config.prompt.max_bytes),
                )
            }
            IndexingFragment {
                index_uid,
                embedder,
                fragment,
            }
            | SearchFragment {
                index_uid,
                embedder,
                fragment,
            } => {
                let index = self.resolve_index(index_uid)?;
                let txn = index.inner.read_txn()?;
                let config = index
                    .inner
                    .embedding_configs()
                    .embedding_configs(&txn)?
                    .into_iter()
                    .find(|c| &c.name == embedder)
                    .ok_or_else(|| Error::EmbedderNotFound(embedder.clone()))?;
                let value = if matches!(request.template, IndexingFragment { .. }) {
                    config.config.embedder_options.indexing_fragment(fragment)
                } else {
                    config.config.embedder_options.search_fragment(fragment)
                };
                (
                    value
                        .cloned()
                        .ok_or_else(|| Error::Internal(format!("Missing fragment {fragment}")))?,
                    None,
                )
            }
        };
        let prompt = if fragment {
            None
        } else {
            Some(
                Prompt::new(
                    template.as_str().expect("string template").into(),
                    max_bytes,
                )
                .map_err(render_error)?,
            )
        };
        let json = if fragment {
            Some(
                JsonTemplate::new(template.clone())
                    .map_err(|e| Error::Internal(e.parsing_error("template")))?,
            )
        } else {
            None
        };
        let rendered = if let Some(input) = &request.input {
            let mut metadata = milli::FieldIdMapWithMetadata::empty();
            let document = match input {
                RenderInput::InlineDocument { inline } => Some(Value::Object(inline.clone())),
                RenderInput::IndexDocument { index_uid, id } => {
                    let index = self.resolve_index(index_uid)?;
                    let id = milli::documents::validate_document_id_value(id.clone())
                        .map_err(render_error)?;
                    let txn = index.inner.read_txn()?;
                    metadata = index.inner.fields_ids_map_with_metadata(&txn)?;
                    let internal = index
                        .inner
                        .external_documents_ids()
                        .get(&txn, &id)?
                        .ok_or(Error::DocumentNotFound(id))?;
                    Some(index.make_document(
                        &txn,
                        metadata.as_fields_ids_map(),
                        internal,
                        None,
                        true,
                        false,
                    )?)
                }
                RenderInput::InlineSearch { .. } => None,
            };
            if let RenderInput::InlineSearch { inline } = input {
                let json = json.as_ref().ok_or_else(|| {
                    Error::Internal("Search input requires a fragment template".into())
                })?;
                Some(
                    json.render_search(inline.q.as_deref(), inline.media.as_ref())
                        .map_err(|e| Error::Internal(e.rendering_error("template")))?,
                )
            } else {
                let alloc = bumpalo::Bump::new();
                let text = alloc.alloc_str(&serde_json::to_string(&document)?);
                let raw = serde_json::from_str::<&serde_json::value::RawValue>(text)?;
                let doc = bumparaw_collections::RawMap::from_raw_value(raw, &alloc)?;
                if let Some(prompt) = prompt {
                    let fields = RwLock::new(metadata);
                    let global = RefCell::new(milli::GlobalFieldsIdsMap::new(&fields));
                    Some(Value::String(
                        prompt
                            .render_document(None, &doc, &global, &alloc)
                            .map_err(render_error)?
                            .into(),
                    ))
                } else {
                    Some(
                        json.expect("fragment")
                            .render_document(&doc, &alloc)
                            .map_err(|e| Error::Internal(e.rendering_error("template")))?,
                    )
                }
            }
        } else {
            None
        };
        Ok(RenderResponse { template, rendered })
    }
}

fn render_error(error: impl std::fmt::Display) -> Error {
    Error::Internal(error.to_string())
}
