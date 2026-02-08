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
        let fields: Option<Vec<String>> = query
            .fields
            .as_ref()
            .map(|f| f.split(',').map(|s| s.trim().to_string()).collect());
        let doc = idx.get_document_with_fields(document_id, fields.as_deref())?;
        doc.ok_or_else(|| crate::core::error::Error::DocumentNotFound(document_id.to_string()))
    }

    fn get_documents(
        &self,
        index_uid: &str,
        query: &DocumentsQuery,
    ) -> Result<DocumentsResponse> {
        let idx = self.resolve_index(index_uid)?;
        let offset = query.offset.unwrap_or(0) as usize;
        let limit = query.limit.unwrap_or(20) as usize;

        if query.filter.is_some() || query.ids.is_some() || query.sort.is_some() {
            let options = crate::core::search::GetDocumentsOptions {
                offset,
                limit,
                fields: query.fields.as_ref().map(|f| {
                    f.split(',').map(|s| s.trim().to_string()).collect()
                }),
                filter: query.filter.as_ref().map(|f| Value::String(f.clone())),
                ids: query.ids.as_ref().map(|ids_str| {
                    ids_str.split(',').map(|s| s.trim().to_string()).collect()
                }),
                sort: query.sort.as_ref().map(|s| {
                    s.split(',').map(|s| s.trim().to_string()).collect()
                }),
                ..Default::default()
            };
            let result = idx.get_documents_with_options(&options)?;
            return Ok(DocumentsResponse {
                results: result.documents,
                offset: u32::try_from(result.offset).unwrap_or(u32::MAX),
                limit: u32::try_from(result.limit).unwrap_or(u32::MAX),
                total: result.total,
            });
        }

        let result = idx.get_documents(offset, limit)?;
        Ok(DocumentsResponse {
            results: result.documents,
            offset: u32::try_from(result.offset).unwrap_or(u32::MAX),
            limit: u32::try_from(result.limit).unwrap_or(u32::MAX),
            total: result.total,
        })
    }

    fn fetch_documents(
        &self,
        index_uid: &str,
        request: &FetchDocumentsRequest,
    ) -> Result<DocumentsResponse> {
        let idx = self.resolve_index(index_uid)?;
        let options = crate::core::search::GetDocumentsOptions {
            offset: request.offset.unwrap_or(0) as usize,
            limit: request.limit.unwrap_or(20) as usize,
            fields: request.fields.clone(),
            filter: request.filter.as_ref().map(|f| Value::String(f.clone())),
            ..Default::default()
        };
        let result = idx.get_documents_with_options(&options)?;
        Ok(DocumentsResponse {
            results: result.documents,
            offset: u32::try_from(result.offset).unwrap_or(u32::MAX),
            limit: u32::try_from(result.limit).unwrap_or(u32::MAX),
            total: result.total,
        })
    }

    fn add_or_replace_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.add_documents(documents.to_vec(), query.primary_key.as_deref())?;
        self.mutation_task(index_uid, "documentAdditionOrUpdate")
    }

    fn add_or_update_documents(
        &self,
        index_uid: &str,
        documents: &[Value],
        query: &AddDocumentsQuery,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_documents(documents.to_vec(), query.primary_key.as_deref())?;
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
        idx.delete_by_filter(&request.filter)?;
        self.mutation_task(index_uid, "documentDeletion")
    }

    fn delete_documents_by_batch(
        &self,
        index_uid: &str,
        document_ids: &[Value],
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        let ids: Vec<String> = document_ids
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                other => other.to_string(),
            })
            .collect();
        idx.delete_documents(ids)?;
        self.mutation_task(index_uid, "documentDeletion")
    }

    fn delete_all_documents(&self, index_uid: &str) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.clear()?;
        self.mutation_task(index_uid, "documentDeletion")
    }
}
