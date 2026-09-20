//! `Documents` trait implementation for `Engine`.

use serde_json::Value;

use crate::traits::{self, Result};
use crate::types::*;

use super::Engine;

impl traits::Documents for Engine {
    fn get_document(
        &self,
        index_uid: &str,
        document_id: &str,
        query: &DocumentQuery,
    ) -> Result<Value> {
        let idx = self.resolve_index(index_uid)?;
        let options = crate::core::GetDocumentsOptions {
            ids: Some(vec![document_id.to_owned()]),
            fields: query.fields.clone(),
            retrieve_vectors: query.retrieve_vectors,
            limit: 1,
            ..Default::default()
        };
        idx.get_documents_with_options(&options)?
            .documents
            .into_iter()
            .next()
            .ok_or_else(|| crate::core::Error::DocumentNotFound(document_id.into()))
    }

    fn get_documents(&self, index_uid: &str, query: &DocumentsQuery) -> Result<DocumentsResponse> {
        self.fetch_documents(
            index_uid,
            &FetchDocumentsRequest {
                fields: query.fields.clone(),
                filter: query.filter.clone(),
                ids: query.ids.clone(),
                sort: query.sort.clone(),
                offset: query.offset,
                limit: query.limit,
                retrieve_vectors: query.retrieve_vectors,
            },
        )
    }

    fn fetch_documents(
        &self,
        index_uid: &str,
        request: &FetchDocumentsRequest,
    ) -> Result<DocumentsResponse> {
        let ids = request
            .ids
            .as_ref()
            .map(|ids| {
                ids.iter()
                    .cloned()
                    .map(|id| {
                        milli::documents::validate_document_id_value(id)
                            .map_err(|e| crate::core::Error::Internal(e.to_string()))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        let options = crate::core::GetDocumentsOptions {
            offset: request.offset.unwrap_or(0) as usize,
            limit: request.limit.unwrap_or(20) as usize,
            fields: request.fields.clone(),
            filter: request.filter.clone(),
            ids,
            sort: request.sort.clone(),
            retrieve_vectors: request.retrieve_vectors,
        };
        let result = self.inner.get_documents(index_uid, &options)?;
        Ok(DocumentsResponse {
            results: result.documents,
            total: result.total,
            offset: result.offset as u32,
            limit: result.limit as u32,
        })
    }

    fn add_or_replace_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        if query.csv_delimiter.is_some() {
            return Err(crate::core::Error::Internal(
                "csvDelimiter is not valid for JSON documents".into(),
            ));
        }
        idx.index_documents(
            documents.to_vec(),
            query.primary_key.as_deref(),
            false,
            query.skip_creation,
        )?;
        self.mutation_task(index_uid, "documentAdditionOrUpdate")
    }

    fn add_or_update_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        if query.csv_delimiter.is_some() {
            return Err(crate::core::Error::Internal(
                "csvDelimiter is not valid for JSON documents".into(),
            ));
        }
        idx.index_documents(
            documents.to_vec(),
            query.primary_key.as_deref(),
            true,
            query.skip_creation,
        )?;
        self.mutation_task(index_uid, "documentAdditionOrUpdate")
    }

    fn delete_document(&self, index_uid: &str, document_id: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.delete_document(document_id)?;
        self.mutation_task(index_uid, "documentDeletion")
    }

    fn delete_documents_by_filter(
        &self,
        index_uid: &str,
        request: &DeleteDocumentsByFilterRequest,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let filter = self
            .inner
            .resolve_filter(index_uid, Some(&request.filter))?
            .ok_or_else(|| crate::core::Error::InvalidFilter("Empty filter".into()))?;
        idx.delete_by_index_filter(filter)?;
        self.mutation_task(index_uid, "documentDeletion")
    }

    fn delete_documents_by_batch(
        &self,
        index_uid: &str,
        document_ids: &[Value],
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let ids = document_ids
            .iter()
            .cloned()
            .map(|id| {
                milli::documents::validate_document_id_value(id)
                    .map_err(|e| crate::core::Error::Internal(e.to_string()))
            })
            .collect::<Result<Vec<_>>>()?;
        idx.delete_documents(ids)?;
        self.mutation_task(index_uid, "documentDeletion")
    }

    fn delete_all_documents(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.clear()?;
        self.mutation_task(index_uid, "documentDeletion")
    }
}
