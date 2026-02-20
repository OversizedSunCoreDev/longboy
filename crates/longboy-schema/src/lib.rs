use longboy::{ClientToServerSchema, ServerToClientSchema};

pub fn new_client_to_server_schema() -> ClientToServerSchema
{
    ClientToServerSchema {
        name: "Input",
        mapper_port: 8081,
        heartbeat_period: 100,
        port: 8082,
    }
}

pub fn new_server_to_client_schema() -> ServerToClientSchema
{
    ServerToClientSchema {
        name: "State",
        mapper_port: 8080,
        heartbeat_period: 100,
    }
}
