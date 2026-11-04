# Local Grafana monitoring

From the repository root, copy monitoring/.env.example to monitoring/.env and set a unique Grafana password. Run ./start-all.sh to launch the system and infrastructure as separate Compose projects. Use the service control page at http://localhost:8080 to manage individual services or either group.

The app UI is at http://localhost:5173. Grafana is at http://localhost:3000/d/sbe-ts-overview/sbe-ts-system-and-trading, and Prometheus is at http://localhost:9090. Grafana and Prometheus bind to localhost; the app exporter and node exporter are available only inside the Compose network. Service control binds to localhost and requires access to the Docker socket.

Run ./stop-all.sh to stop both projects. Prometheus retains 15 days of metrics. Docker volumes preserve Grafana, Prometheus, and news model data across restarts. Docker Desktop node metrics describe the Linux VM, not necessarily the Windows host.
