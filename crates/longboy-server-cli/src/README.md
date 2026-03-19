
# Longboy Server CLI Architecture

## Broker Module Overview

```mermaid
graph TD
    A[SessionBroker] -->|Manages| B[ClientBroker]
    A -->|Manages| C[ServerBroker]
    B -->|Sends| D[ClientToServerSink]
    C -->|Receives| E[ServerToClientSource]
    A -->|Tracks| F[PlayerSessions]
    A -->|Tracks| G[PlayerIndexFreeList]
```

This diagram illustrates how the broker module manages client connections, routes messages through a queue system, and maintains client state through a registry.
