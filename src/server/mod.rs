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

pub mod services;

use crate::execution::AppExecution;
use crate::test_utils::trailers_layer::TrailersLayer;
use color_eyre::Result;
use log::info;
use metrics::{describe_counter, describe_histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

fn initialize_metrics() {
    describe_counter!("requests", "Incoming requests by FlightSQL endpoint");

    describe_histogram!(
        "get_flight_info_latency_ms",
        metrics::Unit::Milliseconds,
        "Get flight info latency ms"
    );

    describe_histogram!(
        "do_get_fallback_latency_ms",
        metrics::Unit::Milliseconds,
        "Do get fallback latency ms"
    )
}

/// Creates and manages a running FlightSqlServer with a background task
pub struct FlightSqlApp {
    /// channel to send shutdown command
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,

    /// Address the server is listening on
    pub addr: SocketAddr,

    /// handle for the server task
    handle: Option<JoinHandle<Result<(), tonic::transport::Error>>>,
}

impl FlightSqlApp {
    /// create a new app for the flightsql server
    #[allow(dead_code)]
    pub async fn try_new(
        app_execution: AppExecution,
        addr: &str,
        metrics_addr: &str,
    ) -> Result<Self> {
        let flightsql = services::flightsql::FlightSqlServiceImpl::new(app_execution);
        // let OS choose a free port
        let listener = TcpListener::bind(addr).await.unwrap();
        let addr = listener.local_addr().unwrap();

        // prepare the shutdown channel
        let (tx, rx) = tokio::sync::oneshot::channel();

        let server_timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECONDS);

        let shutdown_future = async move {
            rx.await.ok();
        };

        let serve_future = tonic::transport::Server::builder()
            .timeout(server_timeout)
            .layer(TrailersLayer)
            .add_service(flightsql.service())
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                shutdown_future,
            );

        {
            let builder = PrometheusBuilder::new();
            let addr: SocketAddr = metrics_addr.parse()?;
            info!("Listening to metrics on {addr}");
            builder
                .with_http_listener(addr)
                .set_buckets_for_metric(
                    Matcher::Suffix("latency_ms".to_string()),
                    &[
                        1.0, 3.0, 5.0, 10.0, 25.0, 50.0, 75.0, 100.0, 250.0, 500.0, 1000.0, 2500.0,
                        5000.0, 10000.0, 20000.0,
                    ],
                )?
                .install()
                .expect("failed to install metrics recorder/exporter");

            initialize_metrics();
        }

        // Run the server in its own background task
        let handle = tokio::task::spawn(serve_future);

        let app = Self {
            shutdown: Some(tx),
            addr,
            handle: Some(handle),
        };
        Ok(app)
    }

    /// Stops the server and waits for the server to shutdown
    pub async fn shutdown_and_wait(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).expect("server quit early");
        }
        if let Some(handle) = self.handle.take() {
            handle
                .await
                .expect("task join error (panic?)")
                .expect("Server Error found at shutdown");
        }
    }

    pub async fn run_app(self) {
        if let Some(handle) = self.handle {
            handle
                .await
                .expect("Unable to run server task")
                .expect("Server Error found at shutdown");
        } else {
            panic!("Server task not found");
        }
    }
}
