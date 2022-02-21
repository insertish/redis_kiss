use std::env;

use mobc::Pool;
use mobc_redis::{
    redis::{AsyncCommands, ToRedisArgs},
    RedisConnectionManager,
};
use redis::{
    aio::{Connection, PubSub},
    Msg,
};

use serde::{de::DeserializeOwned, Serialize};

#[macro_use]
extern crate lazy_static;

#[derive(PartialEq)]
pub enum PayloadType {
    Json,
    Msgpack,
    Bincode,
}

#[derive(Debug)]
pub enum Error {
    FailedConnection,
    SerFailed,
    DeserFailed,
    PublishFailed,
}

lazy_static! {
    pub static ref REDIS_URI: String =
        env::var("REDIS_URI").unwrap_or_else(|_| "redis://localhost".into());
    static ref REDIS_POOL: Pool<RedisConnectionManager> = {
        let max_open = env::var("REDIS_POOL_MAX_OPEN")
            .unwrap_or_else(|_| "1000".into())
            .parse()
            .unwrap();

        let client = mobc_redis::redis::Client::open(REDIS_URI.to_string()).unwrap();
        let manager = RedisConnectionManager::new(client);
        Pool::builder().max_open(max_open).build(manager)
    };
    pub static ref REDIS_PAYLOAD_TYPE: PayloadType = {
        match env::var("REDIS_PAYLOAD_TYPE")
            .unwrap_or_else(|_| "".into())
            .to_lowercase()
            .as_str()
        {
            "msgpack" => PayloadType::Msgpack,
            "bincode" => PayloadType::Bincode,
            _ => PayloadType::Json,
        }
    };
    pub static ref REDIS_KISS_LENIENT: bool =
        env::var("REDIS_KISS_LENIENT").map_or(true, |v| v == "1");
}

pub async fn get_connection() -> Result<mobc::Connection<RedisConnectionManager>, Error> {
    REDIS_POOL.get().await.map_err(|_| Error::FailedConnection)
}

pub async fn p<
    K: ToRedisArgs + std::marker::Send + std::marker::Sync,
    T: Serialize + std::fmt::Debug,
>(
    channel: K,
    data: T,
) {
    if let Err(error) = publish(channel, data).await {
        eprintln!("Encountered an error in redis-kiss: {error:?}");
    }
}

pub async fn publish<
    K: ToRedisArgs + std::marker::Send + std::marker::Sync,
    T: Serialize + std::fmt::Debug,
>(
    channel: K,
    data: T,
) -> Result<(), Error> {
    let mut conn = REDIS_POOL
        .get()
        .await
        .map_err(|_| Error::FailedConnection)?;

    let _: () = match *REDIS_PAYLOAD_TYPE {
        PayloadType::Json => conn
            .publish(channel, serde_json::json!(data).to_string())
            .await
            .map_err(|_| Error::PublishFailed)?,
        PayloadType::Bincode => conn
            .publish(
                channel,
                bincode::serialize(&data).map_err(|_| Error::SerFailed)?,
            )
            .await
            .map_err(|_| Error::PublishFailed)?,
        PayloadType::Msgpack => conn
            .publish(
                channel,
                rmp_serde::to_vec_named(&data).map_err(|_| Error::SerFailed)?,
            )
            .await
            .map_err(|_| Error::PublishFailed)?,
    };

    Ok(())
}

pub async fn open_pubsub_connection() -> Result<PubSub, Error> {
    redis::Client::open(REDIS_URI.to_string())
        .map_err(|_| Error::FailedConnection)?
        .get_async_connection()
        .await
        .map(Connection::into_pubsub)
        .map_err(|_| Error::FailedConnection)
}

pub fn decode_payload_using<T: DeserializeOwned>(
    msg: &Msg,
    target: &PayloadType,
) -> Result<T, Error> {
    Ok(match target {
        PayloadType::Json => serde_json::from_str(
            &msg.get_payload::<String>()
                .map_err(|_| Error::DeserFailed)?,
        )
        .map_err(|_| Error::DeserFailed)?,
        PayloadType::Msgpack => {
            rmp_serde::from_slice(msg.get_payload_bytes()).map_err(|_| Error::DeserFailed)?
        }
        PayloadType::Bincode => {
            bincode::deserialize(msg.get_payload_bytes()).map_err(|_| Error::DeserFailed)?
        }
    })
}

pub fn decode_payload<T: DeserializeOwned>(msg: &Msg) -> Result<T, Error> {
    let pt = &*REDIS_PAYLOAD_TYPE;
    match decode_payload_using::<T>(msg, pt) {
        Err(error) => {
            if !(*REDIS_KISS_LENIENT) {
                return Err(error);
            }

            if pt != &PayloadType::Json {
                if let Ok(v) = decode_payload_using(msg, &PayloadType::Json) {
                    return Ok(v);
                }
            }

            if pt != &PayloadType::Msgpack {
                if let Ok(v) = decode_payload_using(msg, &PayloadType::Msgpack) {
                    return Ok(v);
                }
            }

            if pt != &PayloadType::Bincode {
                if let Ok(v) = decode_payload_using(msg, &PayloadType::Bincode) {
                    return Ok(v);
                }
            }

            Err(Error::DeserFailed)
        }
        v => v,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::{select, FutureExt, StreamExt};

    #[async_std::test]
    async fn publish_json() {
        crate::p("test", "sus").await;
    }

    #[async_std::test]
    async fn publish_bincode() {
        std::env::set_var("REDIS_PAYLOAD_TYPE", "bincode");
        crate::p("test", "sus").await;
    }

    #[async_std::test]
    async fn publish_msgpack() {
        std::env::set_var("REDIS_PAYLOAD_TYPE", "msgpack");
        crate::p("test", "sus").await;
    }

    #[async_std::test]
    async fn subscribe() {
        if let Ok(mut conn) = crate::open_pubsub_connection().await {
            conn.subscribe("test").await.unwrap();
            conn.subscribe("neat").await.unwrap();
            conn.subscribe("while").await.unwrap();
            crate::p("neat", "epic").await;

            async fn handle_messages(conn: &mut redis::aio::PubSub) {
                loop {
                    if let Some(item) = conn.on_message().next().await {
                        let topic: String = item.get_channel_name().to_string();
                        if let Ok(v) = crate::decode_payload::<String>(&item) {
                            dbg!(&topic, v);
                        } else {
                            eprintln!("deser failed!")
                        }
                    }

                    /*{
                        let mut stream = conn.on_message();
                        while let Some(item) = stream.next().await {
                            let topic: String = item.get_channel_name().to_string();
                            if let Ok(v) = crate::decode_payload::<String>(&item).await {
                                dbg!(&topic, v);

                                if topic == "test" {
                                    break;
                                }
                            } else {
                                eprintln!("deser failed!")
                            }
                        }
                    }*/

                    conn.subscribe("deez").await.unwrap();
                }
            }

            async fn worker_task() {
                crate::p("test", "sus").await;
                async_std::task::sleep(Duration::from_secs(1)).await;
                crate::p("deez", "nuts").await;
                async_std::task::sleep(Duration::from_secs(1)).await;
            }

            select! {
                () = handle_messages(&mut conn).fuse() => {},
                () = worker_task().fuse() => {},
            }
        }
    }
}
