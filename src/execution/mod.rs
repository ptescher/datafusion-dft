// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

//! [`ExecutionContext`]: DataFusion based execution context for running SQL queries
//!

mod stats;
use std::sync::Arc;

pub use stats::{collect_plan_stats, ExecutionStats};

use crate::config::ExecutionConfig;
use crate::extensions::{enabled_extensions, DftSessionStateBuilder};
use color_eyre::eyre::Result;
use datafusion::execution::SendableRecordBatchStream;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::prelude::*;
use datafusion::sql::parser::Statement;
use tokio_stream::StreamExt;
#[cfg(feature = "flightsql")]
use {
    crate::config::FlightSQLConfig, arrow_flight::sql::client::FlightSqlServiceClient,
    tokio::sync::Mutex, tonic::transport::Channel,
};

/// Structure for executing queries either locally or remotely (via FlightSQL)
///
/// This context includes both:
///
/// 1. The configuration of a [`SessionContext`] with  various extensions enabled
///
/// 2. The code for running SQL queries
///
/// The design goals for this module are to serve as an example of how to integrate
/// DataFusion into an application and to provide a simple interface for running SQL queries
/// with the various extensions enabled.
///
/// Thus it is important (eventually) not depend on the code in the app crate
pub struct ExecutionContext {
    session_ctx: SessionContext,
    #[cfg(feature = "flightsql")]
    flightsql_client: Mutex<Option<FlightSqlServiceClient<Channel>>>,
}

impl std::fmt::Debug for ExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionContext").finish()
    }
}

impl ExecutionContext {
    /// Construct a new `ExecutionContext` with the specified configuration
    pub fn try_new(config: &ExecutionConfig) -> Result<Self> {
        let mut builder = DftSessionStateBuilder::new();
        let extensions = enabled_extensions();
        for extension in &extensions {
            builder = extension.register(config, builder)?;
        }

        let state = builder.build()?;
        let mut session_ctx = SessionContext::new_with_state(state);

        // Apply any additional setup to the session context (e.g. registering
        // functions)
        for extension in &extensions {
            extension.register_on_ctx(config, &mut session_ctx)?;
        }

        Ok(Self {
            session_ctx,
            #[cfg(feature = "flightsql")]
            flightsql_client: Mutex::new(None),
        })
    }

    pub fn create_tables(&mut self) -> Result<()> {
        Ok(())
    }

    /// Return the inner DataFusion [`SessionContext`]
    pub fn session_ctx(&self) -> &SessionContext {
        &self.session_ctx
    }

    /// Return a handle to the underlying FlightSQL client, if any
    #[cfg(feature = "flightsql")]
    pub fn flightsql_client(&self) -> &Mutex<Option<FlightSqlServiceClient<Channel>>> {
        &self.flightsql_client
    }

    /// Executes the specified sql string, driving it to completion but discarding any results
    pub async fn execute_sql_and_discard_results(
        &self,
        sql: &str,
    ) -> datafusion::error::Result<()> {
        let mut stream = self.execute_sql(sql).await?;
        // note we don't call collect() to avoid buffering data
        while let Some(maybe_batch) = stream.next().await {
            maybe_batch?; // check for errors
        }
        Ok(())
    }

    /// Create a physical plan from the specified SQL string.  This is useful if you want to store
    /// the plan and collect metrics from it.
    pub async fn create_physical_plan(
        &self,
        sql: &str,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        let df = self.session_ctx.sql(sql).await?;
        df.create_physical_plan().await
    }

    /// Executes the specified sql string, returning the resulting
    /// [`SendableRecordBatchStream`] of results
    pub async fn execute_sql(
        &self,
        sql: &str,
    ) -> datafusion::error::Result<SendableRecordBatchStream> {
        self.session_ctx.sql(sql).await?.execute_stream().await
    }

    /// Executes the a pre-parsed DataFusion [`Statement`], returning the
    /// resulting [`SendableRecordBatchStream`] of results
    pub async fn execute_statement(
        &self,
        statement: Statement,
    ) -> datafusion::error::Result<SendableRecordBatchStream> {
        let plan = self
            .session_ctx
            .state()
            .statement_to_plan(statement)
            .await?;
        self.session_ctx
            .execute_logical_plan(plan)
            .await?
            .execute_stream()
            .await
    }

    /// Create FlightSQL client from users FlightSQL config
    #[cfg(feature = "flightsql")]
    pub async fn create_flightsql_client(&self, config: FlightSQLConfig) -> Result<()> {
        use color_eyre::eyre::eyre;

        let url = Box::leak(config.connection_url.into_boxed_str());
        let channel = Channel::from_static(url).connect().await;
        match channel {
            Ok(c) => {
                let client = FlightSqlServiceClient::new(c);
                let mut guard = self.flightsql_client.lock().await;
                *guard = Some(client);
                Ok(())
            }
            Err(e) => Err(eyre!(
                "Error creating channel for FlightSQL client: {:?}",
                e
            )),
        }
    }
}
