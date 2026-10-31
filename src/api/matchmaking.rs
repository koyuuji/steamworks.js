use napi_derive::napi;

#[napi]
pub mod matchmaking {
    use crate::api::localplayer::PlayerSteamId;
    use napi::bindgen_prelude::{BigInt, Error};
    use std::collections::HashMap;
    use steamworks::LobbyId;
    use tokio::sync::oneshot;

    #[napi]
    pub enum LobbyType {
        Private,
        FriendsOnly,
        Public,
        Invisible,
    }

    #[napi]
    pub enum LobbyComparison {
        Equal,
        NotEqual,
        GreaterThan,
        GreaterThanEqualTo,
        LessThan,
        LessThanEqualTo,
    }

    #[napi]
    pub enum LobbyDistance {
        Close,
        /// The searcher's own region and nearby ones. Steam's default.
        Default,
        /// About half-way around the globe.
        Far,
        Worldwide,
    }

    /// Lobby data `key` must equal `value`, or with `exclude` must not.
    #[napi(object)]
    pub struct LobbyStringFilter {
        pub key: String,
        pub value: String,
        pub exclude: Option<bool>,
    }

    /// Lobby data `key`, read as a number, compared with `value`. `Equal` when no comparison is given.
    #[napi(object)]
    pub struct LobbyNumberFilter {
        pub key: String,
        pub value: i32,
        pub comparison: Option<LobbyComparison>,
    }

    /// Sorts results by how close lobby data `key` is to `value`. Excludes nothing.
    #[napi(object)]
    pub struct LobbyNearFilter {
        pub key: String,
        pub value: i32,
    }

    /// What `getLobbies` asks Steam for. Every field is optional.
    #[napi(object)]
    pub struct LobbyListFilter {
        pub strings: Option<Vec<LobbyStringFilter>>,
        pub numbers: Option<Vec<LobbyNumberFilter>>,
        pub near: Option<Vec<LobbyNearFilter>>,
        /// Only lobbies with at least this many free places.
        pub open_slots: Option<u32>,
        pub distance: Option<LobbyDistance>,
        /// At most this many lobbies.
        pub count: Option<u32>,
    }

    #[napi]
    pub struct Lobby {
        pub id: BigInt,
        lobby_id: LobbyId,
    }

    #[napi]
    impl Lobby {
        #[napi]
        pub async fn join(&self) -> Result<Lobby, Error> {
            join_lobby(self.id.clone()).await
        }

        #[napi]
        pub fn leave(&self) {
            let client = crate::client::get_client();
            client.matchmaking().leave_lobby(self.lobby_id);
        }

        #[napi]
        pub fn open_invite_dialog(&self) {
            let client = crate::client::get_client();
            client.friends().activate_invite_dialog(self.lobby_id);
        }

        #[napi]
        pub fn get_member_count(&self) -> usize {
            let client = crate::client::get_client();
            client.matchmaking().lobby_member_count(self.lobby_id)
        }

        #[napi]
        pub fn get_member_limit(&self) -> Option<usize> {
            let client = crate::client::get_client();
            client.matchmaking().lobby_member_limit(self.lobby_id)
        }

        #[napi]
        pub fn get_members(&self) -> Vec<PlayerSteamId> {
            let client = crate::client::get_client();
            client
                .matchmaking()
                .lobby_members(self.lobby_id)
                .into_iter()
                .map(PlayerSteamId::from_steamid)
                .collect()
        }

        #[napi]
        pub fn get_owner(&self) -> PlayerSteamId {
            let client = crate::client::get_client();
            PlayerSteamId::from_steamid(client.matchmaking().lobby_owner(self.lobby_id))
        }

        #[napi]
        pub fn set_joinable(&self, joinable: bool) -> bool {
            let client = crate::client::get_client();
            client
                .matchmaking()
                .set_lobby_joinable(self.lobby_id, joinable)
        }

        #[napi]
        pub fn get_data(&self, key: String) -> Option<String> {
            let client = crate::client::get_client();
            client
                .matchmaking()
                .lobby_data(self.lobby_id, &key)
                .map(|s| s.to_string())
        }

        #[napi]
        pub fn set_data(&self, key: String, value: String) -> bool {
            let client = crate::client::get_client();
            client
                .matchmaking()
                .set_lobby_data(self.lobby_id, &key, &value)
        }

        #[napi]
        pub fn delete_data(&self, key: String) -> bool {
            let client = crate::client::get_client();
            client.matchmaking().delete_lobby_data(self.lobby_id, &key)
        }

        /// Get an object containing all the lobby data
        #[napi]
        pub fn get_full_data(&self) -> HashMap<String, String> {
            let client = crate::client::get_client();

            let mut data = HashMap::new();

            let count = client.matchmaking().lobby_data_count(self.lobby_id);
            for i in 0..count {
                let maybe_lobby_data = client.matchmaking().lobby_data_by_index(self.lobby_id, i);

                if let Some((key, value)) = maybe_lobby_data {
                    data.insert(key, value);
                }
            }

            data
        }

        /// Merge current lobby data with provided data in a single batch
        /// @returns true if all data was set successfully
        #[napi]
        pub fn merge_full_data(&self, data: HashMap<String, String>) -> bool {
            let matchmaking = crate::client::get_client().matchmaking();
            data.iter()
                .map(|(key, value)| matchmaking.set_lobby_data(self.lobby_id, key, value))
                .all(|x| x)
        }
    }

    #[napi]
    pub async fn create_lobby(lobby_type: LobbyType, max_members: u32) -> Result<Lobby, Error> {
        let client = crate::client::get_client();

        let (tx, rx) = oneshot::channel();

        client.matchmaking().create_lobby(
            match lobby_type {
                LobbyType::Private => steamworks::LobbyType::Private,
                LobbyType::FriendsOnly => steamworks::LobbyType::FriendsOnly,
                LobbyType::Public => steamworks::LobbyType::Public,
                LobbyType::Invisible => steamworks::LobbyType::Invisible,
            },
            max_members,
            |result| {
                tx.send(result).unwrap();
            },
        );

        rx.await
            .unwrap()
            .map(|lobby_id| Lobby {
                id: BigInt::from(lobby_id.raw()),
                lobby_id,
            })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    #[napi]
    pub async fn join_lobby(lobby_id: BigInt) -> Result<Lobby, Error> {
        let client = crate::client::get_client();

        let (tx, rx) = oneshot::channel();

        client.matchmaking().join_lobby(
            steamworks::LobbyId::from_raw(lobby_id.get_u64().1),
            |result| {
                tx.send(result).unwrap();
            },
        );

        rx.await
            .unwrap()
            .map(|lobby_id| Lobby {
                id: BigInt::from(lobby_id.raw()),
                lobby_id,
            })
            .map_err(|_| Error::from_reason("Failed to join lobby".to_string()))
    }

    /// Search for lobbies, filtered or not. Steam runs one search at a time and a new one cancels the
    /// last. Without a `distance` filter it returns only the searcher's region and nearby ones.
    #[napi]
    pub async fn get_lobbies(filter: Option<LobbyListFilter>) -> Result<Vec<Lobby>, Error> {
        let client = crate::client::get_client();

        // Steam holds filters until the next request and then clears them, so they go on just before it.
        if let Some(filter) = &filter {
            apply_filter(&client, filter)?;
        }

        let (tx, rx) = oneshot::channel();

        client.matchmaking().request_lobby_list(|lobbies| {
            tx.send(lobbies).unwrap();
        });

        rx.await
            .unwrap()
            .map(|lobbies| {
                lobbies
                    .iter()
                    .map(|lobby_id| Lobby {
                        id: BigInt::from(lobby_id.raw()),
                        lobby_id: *lobby_id,
                    })
                    .collect()
            })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    fn key(key: &str) -> Result<steamworks::LobbyKey<'_>, Error> {
        steamworks::LobbyKey::try_new(key)
            .map_err(|e| Error::from_reason(format!("{}: {}", e, key)))
    }

    fn apply_filter(client: &steamworks::Client, filter: &LobbyListFilter) -> Result<(), Error> {
        let strings = filter
            .strings
            .as_ref()
            .map(|filters| {
                filters
                    .iter()
                    .map(|f| {
                        let kind = if f.exclude.unwrap_or(false) {
                            steamworks::StringFilterKind::Exclude
                        } else {
                            steamworks::StringFilterKind::Include
                        };
                        Ok(steamworks::StringFilter(
                            key(&f.key)?,
                            f.value.as_str(),
                            kind,
                        ))
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })
            .transpose()?;

        let numbers = filter
            .numbers
            .as_ref()
            .map(|filters| {
                filters
                    .iter()
                    .map(|f| {
                        let comparison = match f.comparison.as_ref() {
                            None | Some(LobbyComparison::Equal) => {
                                steamworks::ComparisonFilter::Equal
                            }
                            Some(LobbyComparison::NotEqual) => {
                                steamworks::ComparisonFilter::NotEqual
                            }
                            Some(LobbyComparison::GreaterThan) => {
                                steamworks::ComparisonFilter::GreaterThan
                            }
                            Some(LobbyComparison::GreaterThanEqualTo) => {
                                steamworks::ComparisonFilter::GreaterThanEqualTo
                            }
                            Some(LobbyComparison::LessThan) => {
                                steamworks::ComparisonFilter::LessThan
                            }
                            Some(LobbyComparison::LessThanEqualTo) => {
                                steamworks::ComparisonFilter::LessThanEqualTo
                            }
                        };
                        Ok(steamworks::NumberFilter(key(&f.key)?, f.value, comparison))
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })
            .transpose()?;

        let near_value = filter
            .near
            .as_ref()
            .map(|filters| {
                filters
                    .iter()
                    .map(|f| Ok(steamworks::NearFilter(key(&f.key)?, f.value)))
                    .collect::<Result<Vec<_>, Error>>()
            })
            .transpose()?;

        let distance = filter.distance.as_ref().map(|distance| match distance {
            LobbyDistance::Close => steamworks::DistanceFilter::Close,
            LobbyDistance::Default => steamworks::DistanceFilter::Default,
            LobbyDistance::Far => steamworks::DistanceFilter::Far,
            LobbyDistance::Worldwide => steamworks::DistanceFilter::Worldwide,
        });

        client
            .matchmaking()
            .set_lobby_list_filter(steamworks::LobbyListFilter {
                string: strings,
                number: numbers,
                near_value,
                open_slots: filter.open_slots.map(|n| n.min(u8::MAX as u32) as u8),
                distance,
                count: filter.count.map(u64::from),
            });

        Ok(())
    }
}
