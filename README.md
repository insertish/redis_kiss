# redis_kiss

Note: this now also exposes access to the Redis connection pool.

This is a pretty simple library intended for just consuming and producing messages for Redis PubSub, it does not follow "good programming practice" in the slightest but it does work and does simplify the work required.

The library manages a global pool of Redis connections and creates listener connections on the fly (due to some technical limitations).

## Example

Publish something to Redis:

```rust
// Handle the Result<_, _>
redis_kiss::publish("channel", "data").await?;

// Log the error instead
redis_kiss::p("channel", "data").await;
```

Subscribe to Redis (as usual, [see Redis documentation for more info](https://docs.rs/redis/latest/redis/aio/struct.PubSub.html)):

```rust
// Create a new PubSub connection
let mut conn = redis_kiss::open_pubsub_connection().await?;

// Subscribe to something
conn.subscribe("test").await?;

// Handle incoming messages
let mut stream = conn.on_message();
while let Some(item) = stream.next().await {
    // Use Redis crate API as usual
    let channel = item.get_channel_name().to_string();

    // Decode the message using previously specified options
    // into a certain DeserializeOwned data type T, in this case String
    let payload: String = redis_kiss::decode_payload(&item).await?;

    // You can also choose a specific type on the fly
    use redis_kiss::PayloadType;
    let payload: String = redis_kiss::decode_payload(&item, &PayloadType::Msgpack).await?;

    // Do something with channel and payload
}
```

## Environment Variables

The library is provided with options through the environment.

| Name                  | Description                                                                                                  | Default             |
| --------------------- | ------------------------------------------------------------------------------------------------------------ | ------------------- |
| `REDIS_URI`           | URI of the Redis server                                                                                      | `redis://localhost` |
| `REDIS_POOL_MAX_OPEN` | Maximum number of connections in Redis pool (for publishing data)                                            | `1000`              |
| `REDIS_PAYLOAD_TYPE`  | Data type to use when publishing data to Redis, either `json`, `msgpack` or `bincode`                        | `json`              |
| `REDIS_KISS_LENIENT`  | Whether to try to automatically decode other payload types if we fail to deserialize it using the chosen one | `1`                 |
