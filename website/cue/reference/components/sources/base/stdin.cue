package metadata

base: components: sources: stdin: configuration: {
	decoding: {
		description: "Decoding configuration."
		required:    false
		type: object: {
			default: codec: "bytes"
			options: codec: {
				description: "The decoding method."
				required:    false
				type: string: {
					default: "bytes"
					enum: {
						bytes: "Events containing the byte frame as-is."
						gelf: """
															Events being parsed from a [GELF][gelf] message.

															[gelf]: https://docs.graylog.org/docs/gelf
															"""
						json: "Events being parsed from a JSON string."
						native: """
															Events being parsed from Vector’s [native protobuf format][vector_native_protobuf] ([EXPERIMENTAL][experimental]).

															[vector_native_protobuf]: https://github.com/vectordotdev/vector/blob/master/lib/vector-core/proto/event.proto
															[experimental]: https://vector.dev/highlights/2022-03-31-native-event-codecs
															"""
						native_json: """
															Events being parsed from Vector’s [native JSON format][vector_native_json] ([EXPERIMENTAL][experimental]).

															[vector_native_json]: https://github.com/vectordotdev/vector/blob/master/lib/codecs/tests/data/native_encoding/schema.cue
															[experimental]: https://vector.dev/highlights/2022-03-31-native-event-codecs
															"""
						syslog: "Events being parsed from a Syslog message."
					}
				}
			}
		}
	}
	framing: {
		description: """
			Framing configuration.

			Framing deals with how events are separated when encoded in a raw byte form, where each event is
			a "frame" that must be prefixed, or delimited, in a way that marks where an event begins and
			ends within the byte stream.
			"""
		required: false
		type: object: options: {
			character_delimited: {
				description:   "Options for the character delimited decoder."
				relevant_when: "method = \"character_delimited\""
				required:      true
				type: object: options: {
					delimiter: {
						description: "The character that delimits byte sequences."
						required:    true
						type: uint: {}
					}
					max_length: {
						description: """
																The maximum length of the byte buffer.

																This length does *not* include the trailing delimiter.
																"""
						required: false
						type: uint: default: null
					}
				}
			}
			method: {
				description: "The framing method."
				required:    true
				type: string: enum: {
					bytes:               "Byte frames are passed through as-is according to the underlying I/O boundaries (e.g. split between messages or stream segments)."
					character_delimited: "Byte frames which are delimited by a chosen character."
					length_delimited:    "Byte frames which are prefixed by an unsigned big-endian 32-bit integer indicating the length."
					newline_delimited:   "Byte frames which are delimited by a newline character."
					octet_counting: """
						Byte frames according to the [octet counting][octet_counting] format.

						[octet_counting]: https://tools.ietf.org/html/rfc6587#section-3.4.1
						"""
				}
			}
			newline_delimited: {
				description:   "Options for the newline delimited decoder."
				relevant_when: "method = \"newline_delimited\""
				required:      false
				type: object: options: max_length: {
					description: """
						The maximum length of the byte buffer.

						This length does *not* include the trailing delimiter.
						"""
					required: false
					type: uint: default: null
				}
			}
			octet_counting: {
				description:   "Options for the octet counting decoder."
				relevant_when: "method = \"octet_counting\""
				required:      false
				type: object: options: max_length: {
					description: "The maximum length of the byte buffer."
					required:    false
					type: uint: default: null
				}
			}
		}
	}
	host_key: {
		description: """
			Overrides the name of the log field used to add the current hostname to each event.

			The value will be the current hostname for wherever Vector is running.

			By default, the [global `log_schema.host_key` option][global_host_key] is used.

			[global_host_key]: https://vector.dev/docs/reference/configuration/global-options/#log_schema.host_key
			"""
		required: false
		type: string: default: null
	}
	max_length: {
		description: """
			The maximum buffer size, in bytes, of incoming messages.

			Messages larger than this are truncated.
			"""
		required: false
		type: uint: default: 102400
	}
}
