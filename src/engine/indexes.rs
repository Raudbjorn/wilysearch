//! `Indexes` trait implementation for `Engine`.

use crate::traits::{self, Result};
use crate::types::*;

use super::Engine;
use super::{saturating_u32, usize_to_u64};

impl traits::Indexes for Engine {
    fn list_indexes(&self, query: &PaginationQuery) -> Result<IndexList> {
        let offset = query.offset.unwrap_or(0) as usize;
        let limit = query.limit.unwrap_or(20) as usize;
        let (infos, total) = self.inner.list_indexes_with_pagination(offset, limit)?;
        let results = infos
            .into_iter()
            .map(|info| Index {
                uid: info.uid,
                primary_key: info.primary_key,
                created_at: info.created_at.unwrap_or_default(),
                updated_at: info.updated_at.unwrap_or_default(),
            })
            .collect();
        Ok(IndexList {
            results,
            offset: saturating_u32(offset),
            limit: saturating_u32(limit),
            total: usize_to_u64(total),
        })
    }

    fn get_index(&self, index_uid: &str) -> Result<Index> {
        let idx = self.resolve_index(index_uid)?;
        let pk = idx.primary_key()?;
        let (created_at, updated_at) = self
            .inner
            .get_index_metadata(index_uid)
            .map(|m| (m.created_at, m.updated_at))
            .unwrap_or_default();
        Ok(Index {
            uid: index_uid.to_string(),
            primary_key: pk,
            created_at,
            updated_at,
        })
    }

    fn create_index(&self, request: &CreateIndexRequest) -> Result<TaskInfo> {
        self.inner
            .create_index(&request.uid, request.primary_key.as_deref())?;
        Ok(self.next_task("indexCreation", Some(&request.uid)))
    }

    fn update_index(
        &self,
        index_uid: &str,
        request: &UpdateIndexRequest,
    ) -> Result<TaskInfo> {
        let idx = self.resolve_index(index_uid)?;
        idx.update_primary_key(&request.primary_key)?;
        self.mutation_task(index_uid, "indexUpdate")
    }

    fn swap_indexes(&self, swaps: &[SwapIndexesRequest]) -> Result<TaskInfo> {
        let pairs: Vec<(&str, &str)> = swaps
            .iter()
            .map(|s| (s.indexes[0].as_str(), s.indexes[1].as_str()))
            .collect();
        self.inner.swap_indexes(&pairs)?;
        Ok(self.next_task("indexSwap", None))
    }

    fn delete_index(&self, index_uid: &str) -> Result<TaskInfo> {
        self.inner.delete_index(index_uid)?;
        Ok(self.next_task("indexDeletion", Some(index_uid)))
    }
}
