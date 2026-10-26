# CloudNativePG

Install/manage the CNPG operator separately. cluster.example.yaml defines an example
three-instance database and application database/owner. Choose storageClass, sizing,
TLS policy and backup/recovery configuration for your environment before use.
The application connection uses the cluster's -rw service and injected credentials;
never commit a connection secret. This example deliberately does not claim backups
are configured. See docs/Operations.md.
