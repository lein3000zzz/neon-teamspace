---
title: Observability stack
description: Run Grafana with Prometheus, Tempo, Loki and Pyroscope beside the server, and read its dashboard.
---

The Compose file's `observability` profile runs Grafana's all-in-one OpenTelemetry image,
[`grafana/otel-lgtm`](https://github.com/grafana/docker-otel-lgtm), beside the server. It
is one container with an OpenTelemetry collector in front of Prometheus for metrics, Tempo
for traces, Loki for logs and Pyroscope for profiles, and Grafana to look at them, with a
dashboard of the server's metrics already provisioned. It suits development and trying
things out: Grafana signs everyone in as an admin, and everything is kept in one volume.
What the server reports is described in [Observability](/docs/architecture/observability/).

## Beside the stack

With the [Docker stack](/docs/self-hosting/install/#with-docker), add the profile to the
command that starts it. Built from a clone:

```sh
docker compose -f deployments/docker-compose.yml -f deployments/docker-compose.build.yml --profile observability up -d --build
```

With the published images, `docker-compose.yml` alone, as the stack started. Grafana is then
at `http://127.0.0.1:3001/` and opens on the dashboard. Prometheus scrapes the server's admin
listener, `server:9090` on the stack's network, every five seconds, so the metrics come in at
once.

Traces and profiles need turning on in `deployments/server.env`, the server's settings, then
the same command again:

```sh
NEON_OBSERVABILITY__TRACING__ENABLED=true
NEON_OBSERVABILITY__PROFILING__MODE=push
NEON_OBSERVABILITY__PROFILING__PUSH__SERVER_ADDRESS=http://otel-lgtm:4040
```

The stack already points the server's trace exporter at the collector, `otel-lgtm:4318`,
through `OTEL_EXPORTER_OTLP_ENDPOINT`, so turning tracing on is enough. For heap profiles
beside CPU profiles, add `NEON_OBSERVABILITY__PROFILING__PROFILES=[cpu, heap]`: the server
image starts jemalloc with its profiler on, as
[heap profiles](/docs/architecture/observability/#heap-profiles) need.

The server writes its logs to standard output, not over OpenTelemetry, so Loki stays empty;
`docker compose -f deployments/docker-compose.yml logs server` shows them.

## Beside `make run`

For a server running from source, `make obs-up` starts the profile's container alone, next
to the development database:

```sh
make obs-up
make run
```

Its ports are published on `127.0.0.1` for the server outside Docker: Grafana on 3001, OTLP
over HTTP on 4318, which is where the server sends traces unless told otherwise, and
Pyroscope on 4040. Prometheus scrapes `host.docker.internal:9090`, the admin listener of
`make run`. To send traces and profiles as well, add to `.env`, which `make run` reads, and
start the server again:

```sh
NEON_OBSERVABILITY__TRACING__ENABLED=true
NEON_OBSERVABILITY__PROFILING__MODE=push
NEON_OBSERVABILITY__PROFILING__PUSH__SERVER_ADDRESS=http://127.0.0.1:4040
```

Docker Desktop passes `host.docker.internal` on to the host's loopback, where the admin
listener is. With Docker Engine on Linux, it is the Docker bridge's address on the host
instead, so the admin listener has to listen there too, for example with
`NEON_OBSERVABILITY__ADMIN__ADDR=172.17.0.1:9090` in `.env`.

`make down` stops the container and keeps its data in the `neon_otel-lgtm` volume.

## The dashboard

The dashboard, **Neon Teamspace server** in the **Neon Teamspace** folder, has five rows.
Its **Instance** menu picks the server: `stack` for the one in the Docker stack, `host` for
`make run`.

| Row | Panels |
|---|---|
| HTTP API | **Rate**, the requests answered per second for each route; **Errors**, the share of each route's requests that failed; **Duration**, the 50th, 95th and 99th percentiles of how long requests took; **In flight**, the requests open at each moment. |
| Jobs | The same four for the job queue's runs, by the job's kind. |
| MCP | **Rate**, **Errors** and the 95th percentile of **Duration** for each [tool](/docs/guides/agents-over-mcp/#tools), and for `list_resources`, `read_resource` and `subscribe`; **Sessions and subscriptions**, how many are open. |
| Webhooks | **Rate**, **Errors** and the 95th percentile of **Duration** for each operation, `deliver` and `receive` among them, beside registering, rotating and testing; **In flight**, the operations under way. |
| Database and queue | **Pool connections**, idle and in use, against the most the pool opens; **Job queue**, the jobs in the queue table by status. |

The panels read the [RED metrics and the gauges](/docs/architecture/observability/#metrics):
`neon_http_*` and `neon_jobs_*` for the first two rows, `neon_mcp_*` with the gauges
`neon_mcp_sessions` and `neon_mcp_subscriptions` for the third, `neon_webhooks_*` for the
fourth, and `neon_db_pool_*` and `neon_jobs_queue_depth` for the last. A 4xx answer counts as
a success, so **Errors** shows the server's own failures. So do MCP's and webhooks': a tool
call refused, a request an incoming webhook refuses and a receiver that answers an error all
count as `ok`. A few requests are enough to see the HTTP row move, for example:

```sh
for i in $(seq 20); do curl -s http://127.0.0.1:3000/api/v1/meta > /dev/null; done
```

The job rows stay empty until the server has jobs to run, the MCP row until a client
connects, and the webhooks row until one is registered or posted to.

## Traces and profiles

In Grafana's **Explore**, the **Tempo** data source finds traces by service: the server's is
`neon`. Each request's span is named after its method and route, such as
`GET /api/v1/meta`; see [Traces](/docs/architecture/observability/#traces). Profiles are in
**Drilldown → Profiles**, or **Explore** with the **Pyroscope** data source, under the
service `neon`.

## Licenses

The stack runs as programs of their own beside Neon Teamspace, which is not linked with any
of them. The `otel-lgtm` image is Apache-2.0, as are Prometheus and the OpenTelemetry
collector in it; Grafana, Loki, Tempo and Pyroscope are AGPL-3.0.
