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

#[cfg(feature = "flightsql")]
pub mod flightsql;
#[cfg(feature = "flightsql")]
pub mod flightsql_benchmarks;
pub mod local;
pub mod sql_utils;

mod local_benchmarks;
mod stats;

pub use stats::{collect_plan_stats, ExecutionStats};

#[cfg(feature = "flightsql")]
use self::flightsql::{FlightSQLClient, FlightSQLContext};
use self::local::ExecutionContext;
use crate::config::AppConfig;
use color_eyre::eyre::Result;
use datafusion::prelude::*;

pub enum AppType {
    Cli,
    Tui,
    FlightSQLServer,
}

/// Provides all core execution functionality for execution queries from either a local
/// `SessionContext` or a remote `FlightSQL` service
pub struct AppExecution {
    context: ExecutionContext,
    #[cfg(feature = "flightsql")]
    flightsql_context: FlightSQLContext,
}

impl AppExecution {
    pub fn new(context: ExecutionContext) -> Self {
        Self {
            context,
            #[cfg(feature = "flightsql")]
            flightsql_context: FlightSQLContext::default(),
        }
    }

    pub fn try_new_from_config(config: AppConfig, app_type: AppType) -> Result<Self> {
        let context = ExecutionContext::try_new(&config.execution, app_type)?;
        #[cfg(feature = "flightsql")]
        let flightsql_context = FlightSQLContext::new(config.flightsql);
        Ok(Self {
            context,
            #[cfg(feature = "flightsql")]
            flightsql_context,
        })
    }

    pub fn execution_ctx(&self) -> &ExecutionContext {
        &self.context
    }

    pub fn session_ctx(&self) -> &SessionContext {
        self.context.session_ctx()
    }

    #[cfg(feature = "flightsql")]
    pub fn flightsql_client(&self) -> &FlightSQLClient {
        self.flightsql_context.client()
    }

    #[cfg(feature = "flightsql")]
    pub fn flightsql_ctx(&self) -> &FlightSQLContext {
        &self.flightsql_context
    }

    #[cfg(feature = "flightsql")]
    pub fn with_flightsql_ctx(&mut self, flightsql_ctx: FlightSQLContext) {
        self.flightsql_context = flightsql_ctx;
    }
}
