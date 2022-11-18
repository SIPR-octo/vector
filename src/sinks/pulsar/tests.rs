#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_config() {
        PulsarSinkConfig::generate_config();
    }
}

#[cfg(feature = "pulsar-integration-tests")]
#[cfg(test)]
mod integration_tests {
    use crate::sinks::pulsar::config::PulsarSinkConfig;
    use crate::sinks::pulsar::sink::PulsarSink;
    use codecs::TextSerializerConfig;
    use futures::StreamExt;
    use pulsar::SubType;
    use std::collections::BTreeMap;

    use crate::event::Value;
    use crate::sinks::VectorSink;
    use crate::test_util::components::assert_sink_compliance;
    use crate::test_util::{
        components::SINK_TAGS, random_lines_with_stream, random_string, trace_init,
    };
    use bytes::Bytes;

    fn pulsar_address() -> String {
        std::env::var("PULSAR_ADDRESS").unwrap_or_else(|_| "pulsar://127.0.0.1:6650".into())
    }

    async fn pulsar_happy_reuse(mut cnf: PulsarSinkConfig) {
        trace_init();

        let prop_1_key = "prop-1-key";
        let prop_1_value = "prop-1-value";
        let num_events = 1_000;
        let (input, events) = random_lines_with_stream(100, num_events, None);
        let prop_key_opt = cnf.properties_key.clone();
        let input_events = events.map(move |mut events| {
            // if a property_key is defined, add some properties!
            if let Some(prop_key) = &prop_key_opt {
                let properties_key = prop_key;
                let mut property_values = BTreeMap::new();
                property_values.insert(
                    prop_1_key.to_owned(),
                    Value::Bytes(Bytes::from(prop_1_value)),
                );
                events.iter_logs_mut().for_each(move |log| {
                    log.insert(properties_key.as_str(), property_values.clone());
                });
                events
            } else {
                events
            }
        });

        let topic = format!("test-{}", random_string(10));

        cnf.topic = topic.clone();

        let pulsar = cnf.create_pulsar_client().await.unwrap();
        let mut consumer = pulsar
            .consumer()
            .with_topic(&topic)
            .with_consumer_name("VectorTestConsumer")
            .with_subscription_type(SubType::Shared)
            .with_subscription("VectorTestSub")
            .with_options(pulsar::consumer::ConsumerOptions {
                read_compacted: Some(false),
                ..Default::default()
            })
            .build::<String>()
            .await
            .unwrap();

        assert_sink_compliance(&SINK_TAGS, async move {
            let sink = PulsarSink::new(pulsar, cnf).unwrap();
            let sink = VectorSink::from_event_streamsink(sink);
            sink.run(input_events).await
        })
        .await
        .expect("Running sink failed");

        for line in input {
            let msg = match consumer.next().await.unwrap() {
                Ok(msg) => msg,
                Err(error) => panic!("{:?}", error),
            };
            consumer.ack(&msg).await.unwrap();
            assert_eq!(String::from_utf8_lossy(&msg.payload.data), line);
        }
    }

    #[tokio::test]
    async fn pulsar_happy() {
        let cnf = PulsarSinkConfig {
            endpoint: pulsar_address(),
            // overriden by test
            topic: "".to_string(),
            key_field: None,
            properties_key: None,
            batch_size: None,
            compression: None,
            encoding: TextSerializerConfig::new().into(),
            auth: None,
            acknowledgements: Default::default(),
        };

        pulsar_happy_reuse(cnf).await
    }
}
