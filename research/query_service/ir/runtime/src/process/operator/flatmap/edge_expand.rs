//
//! Copyright 2021 Alibaba Group Holding Limited.
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! http://www.apache.org/licenses/LICENSE-2.0
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

use std::convert::TryInto;

use graph_proxy::apis::{
    get_graph, DefaultDetails, Direction, DynDetails, GraphElement, GraphObject, Statement, Vertex, ID,
};
use ir_common::generated::algebra as algebra_pb;
use ir_common::KeyId;
use pegasus::api::function::{DynIter, FlatMapFunction, FnResult};

use crate::error::{FnExecError, FnGenError, FnGenResult};
use crate::process::operator::flatmap::FlatMapFuncGen;
use crate::process::record::{Record, RecordExpandIter, RecordPathExpandIter};

pub struct EdgeExpandOperator<E: Into<GraphObject>> {
    start_v_tag: Option<KeyId>,
    edge_or_end_v_tag: Option<KeyId>,
    stmt: Box<dyn Statement<ID, E>>,
    getv_flag: bool,
}

impl<E: Into<GraphObject> + 'static> FlatMapFunction<Record, Record> for EdgeExpandOperator<E> {
    type Target = DynIter<Record>;

    fn exec(&self, input: Record) -> FnResult<Self::Target> {
        let entry = input
            .get(self.start_v_tag)
            .ok_or(FnExecError::get_tag_error("get start_v failed"))?;
        if let Some(v) = entry.as_graph_vertex() {
            let id = v.id();
            let iter = self.stmt.exec(id)?;
            if self.getv_flag {
                let neighbors_iter = iter.map(|e| match e.into() {
                    GraphObject::E(e) => Vertex::new(
                        e.get_other_id(),
                        e.get_other_label().cloned(),
                        DynDetails::new(DefaultDetails::default()),
                    ),
                    _ => {
                        unreachable!()
                    }
                });
                Ok(Box::new(RecordExpandIter::new(
                    input,
                    self.edge_or_end_v_tag.as_ref(),
                    Box::new(neighbors_iter),
                )))
            } else {
                Ok(Box::new(RecordExpandIter::new(input, self.edge_or_end_v_tag.as_ref(), iter)))
            }
        } else if let Some(graph_path) = entry.as_graph_path() {
            let path_end = graph_path
                .get_path_end()
                .ok_or(FnExecError::unexpected_data_error("Get path_end failed in path expand"))?;
            let id = path_end.id();
            let iter = self.stmt.exec(id)?;
            let curr_path = graph_path.clone();
            Ok(Box::new(RecordPathExpandIter::new(input, curr_path, iter)))
        } else {
            Err(FnExecError::unexpected_data_error(&format!(
                "Cannot Expand from current entry {:?}",
                entry
            )))?
        }
    }
}

impl FlatMapFuncGen for algebra_pb::EdgeExpand {
    fn gen_flat_map(
        self,
    ) -> FnGenResult<Box<dyn FlatMapFunction<Record, Record, Target = DynIter<Record>>>> {
        let graph = get_graph().ok_or(FnGenError::NullGraphError)?;
        let start_v_tag = self
            .v_tag
            .map(|v_tag| v_tag.try_into())
            .transpose()?;
        let edge_or_end_v_tag = self
            .alias
            .map(|alias| alias.try_into())
            .transpose()?;
        let direction_pb: algebra_pb::edge_expand::Direction =
            unsafe { ::std::mem::transmute(self.direction) };
        let direction = Direction::from(direction_pb);
        let query_params = self.params.try_into()?;
        let is_edge = self.is_edge;
        debug!(
            "Runtime expand operator of edge with start_v_tag {:?}, edge_tag {:?}, direction {:?}, query_params {:?}, is_edge {:?}", 
            start_v_tag, edge_or_end_v_tag, direction, query_params, is_edge
        );
        if is_edge {
            // Expand edges.
            let stmt = graph.prepare_explore_edge(direction, &query_params)?;
            let edge_expand_operator =
                EdgeExpandOperator { start_v_tag, edge_or_end_v_tag, stmt, getv_flag: false };
            Ok(Box::new(edge_expand_operator))
        } else {
            if query_params.filter.is_some() {
                // Expand vertices with filters on edges.
                // This can be regarded as a combination of EdgeExpand (with is_edge = true) + GetV
                let stmt = graph.prepare_explore_edge(direction, &query_params)?;
                let edge_expand_operator =
                    EdgeExpandOperator { start_v_tag, edge_or_end_v_tag, stmt, getv_flag: true };
                Ok(Box::new(edge_expand_operator))
            } else {
                // Expand vertices without any filters
                let stmt = graph.prepare_explore_vertex(direction, &query_params)?;
                let edge_expand_operator =
                    EdgeExpandOperator { start_v_tag, edge_or_end_v_tag, stmt, getv_flag: false };
                Ok(Box::new(edge_expand_operator))
            }
        }
    }
}
