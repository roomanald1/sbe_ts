# Local Grafana monitoring

From the repository root, copy monitoring/.env.example to monitoring/.env and set a unique Grafana password. Then run ./start-all.sh to start the backend services, React UI, and monitoring together.

The UI is at http://localhost:5173. Grafana is at http://localhost:3000/d/sbe-ts-overview/sbe-ts-system-and-trading, and Prometheus is at http://localhost:9090. Grafana and Prometheus bind to localhost; the app exporter and node exporter are available only inside the Compose network.

Run ./stop-all.sh from another terminal, or press Ctrl+C in the start-all terminal, to stop the stack. Prometheus retains 15 days of metrics. Docker volumes preserve Grafana and Prometheus data across restarts. Docker Desktop node metrics describe the Linux VM, not necessarily the Windows host.
