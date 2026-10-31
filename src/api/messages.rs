// ISteamNetworkingMessages: messages to a Steam user with no connection to manage, carried over
// Valve's relay. Sending to a user opens a session with them, and accepts theirs, implicitly; a
// session request from anyone else is accepted only if they are in the set `acceptSessionsFrom`
// last named. Steam delivers both session callbacks inside `runCallbacks`, on the JavaScript thread.

use napi::bindgen_prelude::BigInt;
use napi::threadsafe_function::{ErrorStrategy, ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use std::collections::HashSet;
use std::sync::{Mutex, Once};

use messages::SessionFailure;

struct Sessions {
    accepted: HashSet<u64>,
    on_failed: Option<ThreadsafeFunction<SessionFailure, ErrorStrategy::Fatal>>,
}

lazy_static! {
    static ref SESSIONS: Mutex<Sessions> = Mutex::new(Sessions {
        accepted: HashSet::new(),
        on_failed: None,
    });
}

static REGISTER: Once = Once::new();

/// Once per process, on first use: steamworks-rs hands back no handle to take them away again.
fn register_session_callbacks() {
    REGISTER.call_once(|| {
        let messages = crate::client::get_client().networking_messages();

        // A request that is not accepted here is refused when it is dropped.
        messages.session_request_callback(|request| {
            let accepted = request.remote().steam_id().is_some_and(|id| {
                SESSIONS
                    .lock()
                    .is_ok_and(|sessions| sessions.accepted.contains(&id.raw()))
            });
            if accepted {
                request.accept();
            }
        });

        messages.session_failed_callback(|info| {
            let steam_id64 = info
                .identity_remote()
                .and_then(|identity| identity.steam_id())
                .map(|id| BigInt::from(id.raw()));
            // `end_reason` panics on a code steamworks-rs does not know, and a panic must not
            // unwind out of Steam's callback dispatch.
            let end_reason =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| info.end_reason()))
                    .ok()
                    .flatten()
                    .map(i32::from);
            if let Ok(sessions) = SESSIONS.lock() {
                if let Some(handler) = &sessions.on_failed {
                    handler.call(
                        SessionFailure {
                            steam_id64,
                            end_reason,
                        },
                        ThreadsafeFunctionCallMode::NonBlocking,
                    );
                }
            }
        });
    });
}

#[napi]
pub mod messages {
    use napi::bindgen_prelude::{BigInt, Buffer};
    use napi::threadsafe_function::{ErrorStrategy, ThreadSafeCallContext, ThreadsafeFunction};
    use napi::{Error, JsFunction};
    use steamworks::networking_types::{
        NetworkingAvailabilityResult, NetworkingIdentity, SendFlags as SteamSendFlags,
    };
    use steamworks::SteamId;

    use crate::api::localplayer::PlayerSteamId;

    /// Bits for `sendToUser`'s `flags`, combined with `|`. An unreliable message is at most 1200 bytes.
    #[napi]
    pub enum SendFlags {
        Unreliable = 0,
        NoNagle = 1,
        NoDelay = 4,
        Reliable = 8,
        /// Reopen a session the peer has closed rather than fail the send.
        AutoRestartBrokenSession = 32,
    }

    #[napi(object)]
    pub struct ReceivedMessage {
        pub data: Buffer,
        pub steam_id: PlayerSteamId,
        pub channel: i32,
    }

    #[napi(object)]
    pub struct SessionFailure {
        /// The peer, when Steam named one.
        pub steam_id64: Option<BigInt>,
        /// An `ESteamNetConnectionEnd` code, when Steam gave one this build knows.
        pub end_reason: Option<i32>,
    }

    /// Each field is Steam's availability name: `Current` is ready, and `Waiting` or `Attempting` is
    /// on its way; anything else is a failure.
    #[napi(object)]
    pub struct RelayStatus {
        pub availability: String,
        pub network_config: String,
        pub any_relay: String,
        pub ping_measurement_in_progress: bool,
        pub debug: String,
    }

    /// Send `data` to a Steam user. `flags` defaults to `Unreliable`, `channel` to 0.
    #[napi]
    pub fn send_to_user(
        steam_id64: BigInt,
        data: Buffer,
        flags: Option<i32>,
        channel: Option<u32>,
    ) -> Result<(), Error> {
        let client = crate::client::get_client();
        let user = NetworkingIdentity::new_steam_id(SteamId::from_raw(steam_id64.get_u64().1));
        let flags =
            SteamSendFlags::from_bits_truncate(flags.unwrap_or(SendFlags::Unreliable as i32));
        client
            .networking_messages()
            .send_message_to_user(user, flags, &data, channel.unwrap_or(0))
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Up to `max` messages waiting on `channel`, oldest first. One from anything but a Steam user is
    /// dropped.
    #[napi]
    pub fn receive(channel: u32, max: u32) -> Vec<ReceivedMessage> {
        if max == 0 {
            return Vec::new();
        }
        let client = crate::client::get_client();
        client
            .networking_messages()
            .receive_messages_on_channel(channel, max as usize)
            .into_iter()
            .filter_map(|message| {
                let steam_id = message.identity_peer().steam_id()?;
                Some(ReceivedMessage {
                    data: message.data().to_vec().into(),
                    steam_id: PlayerSteamId::from_steamid(steam_id),
                    channel: message.channel(),
                })
            })
            .collect()
    }

    /// Replace the set of users whose session requests are accepted; anyone else's is refused. Only a
    /// first message from someone not yet sent to needs this, since sending accepts implicitly.
    #[napi]
    pub fn accept_sessions_from(steam_ids64: Vec<BigInt>) {
        super::register_session_callbacks();
        let accepted = steam_ids64.iter().map(|id| id.get_u64().1).collect();
        if let Ok(mut sessions) = super::SESSIONS.lock() {
            sessions.accepted = accepted;
        }
    }

    /// Called with each session Steam gives up on. Replaces any handler set before.
    #[napi]
    pub fn on_session_failed(
        #[napi(ts_arg_type = "(failure: SessionFailure) => void")] handler: JsFunction,
    ) -> Result<(), Error> {
        let handler: ThreadsafeFunction<SessionFailure, ErrorStrategy::Fatal> = handler
            .create_threadsafe_function(0, |ctx: ThreadSafeCallContext<SessionFailure>| {
                Ok(vec![ctx.value])
            })?;
        super::register_session_callbacks();
        if let Ok(mut sessions) = super::SESSIONS.lock() {
            sessions.on_failed = Some(handler);
        }
        Ok(())
    }

    /// Never connect directly: every session goes through Valve's relay, so no peer learns another's
    /// IP address. Call before anything is sent; it applies to sessions opened after it.
    #[napi]
    pub fn set_relay_only() -> bool {
        let _client = crate::client::get_client();
        unsafe {
            let utils = steamworks::sys::SteamAPI_SteamNetworkingUtils_SteamAPI_v004();
            !utils.is_null()
                && steamworks::sys::SteamAPI_ISteamNetworkingUtils_SetGlobalConfigValueInt32(
                    utils,
                    steamworks::sys::ESteamNetworkingConfigValue::k_ESteamNetworkingConfig_P2P_Transport_ICE_Enable,
                    steamworks::sys::k_nSteamNetworkingConfig_P2P_Transport_ICE_Enable_Disable,
                )
        }
    }

    /// Start reaching the relay network now, rather than on the first message, so the first session
    /// opens sooner. Also retries after a failure.
    #[napi]
    pub fn init_relay_network_access() {
        crate::client::get_client()
            .networking_utils()
            .init_relay_network_access();
    }

    #[napi]
    pub fn relay_status() -> RelayStatus {
        let client = crate::client::get_client();
        // steamworks-rs panics on an availability value it does not know.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let status = client.networking_utils().detailed_relay_network_status();
            RelayStatus {
                availability: describe(status.availability()),
                network_config: describe(status.network_config()),
                any_relay: describe(status.any_relay()),
                ping_measurement_in_progress: status.is_ping_measurement_in_progress(),
                debug: status.debugging_message().to_string(),
            }
        }))
        .unwrap_or_else(|_| RelayStatus {
            availability: "Unknown".to_string(),
            network_config: "Unknown".to_string(),
            any_relay: "Unknown".to_string(),
            ping_measurement_in_progress: false,
            debug: String::new(),
        })
    }

    fn describe(availability: NetworkingAvailabilityResult) -> String {
        match availability {
            Ok(availability) => format!("{:?}", availability),
            Err(error) => format!("{:?}", error),
        }
    }
}
