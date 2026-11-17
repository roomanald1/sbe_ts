# Local Grafana monitoring

From the repository root, copy monitoring/.env.example to monitoring/.env and set a unique Grafana password. Run ./start-all.sh to launch the system and infrastructure as separate Compose projects. Use the service control page at http://localhost:8080 to manage individual services or either group.

The app UI is at http://localhost:5173. Grafana is at http://localhost:3000/d/sbe-ts-overview/sbe-ts-system-and-trading, and Prometheus is at http://localhost:9090. Grafana and Prometheus bind to localhost; the app exporter and node exporter are available only inside the Compose network. Service control binds to localhost and requires access to the Docker socket.

The Runtime Performance section includes per-worker availability, RSS and RSS as a percentage of the configured memory limit, request throughput/saturation, p95 latency, errors/timeouts, Insights subscription state and event age, the installed live profile-set version, and Scout Insights local-route p50/p95 latency. The Scout Insights latency histogram measures local handler dispatch and excludes upstream stream delivery. Use the role and strategy filters to inspect the live and Scout workers. Scout counts are read from cached per-worker metrics; the exporter does not build the full Scout outcome report. Confirm all 34 worker series are present after deployment.

Run ./stop-all.sh to stop both projects. Prometheus retains 15 days of metrics. Docker volumes preserve Grafana, Prometheus, and news model data across restarts. Docker Desktop node metrics describe the Linux VM, not necessarily the Windows host.
