//! Read request catalog. Endpoint, operation, variables and selection travel together.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReadOperation {
    Enrichment,
    Wallboxes,
    CompletedHistory,
    LiveHistory,
    ChargingSchedule,
    DepartureSchedules,
    OtaDetails,
    Baseline,
}

pub(super) struct ReadRequest {
    pub operation: &'static str,
    pub url: &'static str,
    pub query: &'static str,
    pub variables: serde_json::Value,
}

impl ReadOperation {
    pub fn request(self, vehicle_id: &str) -> ReadRequest {
        let (operation, url, query, variable) = match self {
            Self::Enrichment => (
                "getUserInfo",
                GATEWAY_URL,
                r#"
        query getUserInfo {
          currentUser {
            vehicles {
              id
              vehicle {
                mobileConfiguration {
                  trimOption          { optionId optionName }
                  exteriorColorOption { optionId optionName }
                  interiorColorOption { optionId optionName }
                }
                vehicleState {
                  supportedFeatures {
                    name
                    status
                  }
                }
              }
            }
          }
        }
    "#,
                None,
            ),
            Self::Wallboxes => (
                "getRegisteredWallboxes",
                CHRG_URL,
                r#"
        query getRegisteredWallboxes {
          getRegisteredWallboxes {
            wallboxId
            name
            latitude
            longitude
            maxPower
            model
            serialNumber
            softwareVersion
            linked
          }
        }
    "#,
                None,
            ),
            Self::CompletedHistory => (
                "getCompletedSessionSummaries",
                CHRG_URL,
                r#"
        query getCompletedSessionSummaries {
          getCompletedSessionSummaries {
            transactionId
            startInstant
            endInstant
            totalEnergyKwh
            rangeAddedKm
            vendor
            paidTotal
            chargerType
            currencyCode
            city
            vehicleId
            vehicleName
            isPublic
            isHomeCharger
            isRoamingNetwork
            meta {
              transactionIdGroupingKey
              dataSources
            }
          }
        }
    "#,
                None,
            ),
            Self::LiveHistory => (
                "getLiveSessionHistory",
                CHRG_URL,
                r#"
    query getLiveSessionHistory($vehicleId: ID!) {
      getLiveSessionHistory(vehicleId: $vehicleId) {
        chartData { kw time }
      }
    }
"#,
                Some("vehicleId"),
            ),
            Self::ChargingSchedule => (
                "GetChargingSchedule",
                GATEWAY_URL,
                r#"
        query GetChargingSchedule($vehicleId: String!) {
          getVehicle(id: $vehicleId) {
            chargingSchedules {
              enabled
              startTime
              duration
              amperage
              location {
                latitude
                longitude
              }
              weekDays
            }
          }
        }
    "#,
                Some("vehicleId"),
            ),
            Self::DepartureSchedules => (
                "getDepartureSchedules",
                GATEWAY_URL,
                r#"
        query getDepartureSchedules($vehicleId: String!) {
          getDepartureSchedules(vehicleId: $vehicleId) {
            id
            name
            enabled
            occurrence
            preconditioningSettings
          }
        }
    "#,
                Some("vehicleId"),
            ),
            Self::OtaDetails => (
                "getOTAUpdateDetails",
                GATEWAY_URL,
                r#"
        query getOTAUpdateDetails($vehicleId: String!) {
          getOTAUpdateDetails(vehicleId: $vehicleId) {
            releaseNotesUrl
          }
        }
    "#,
                Some("vehicleId"),
            ),
            Self::Baseline => (
                "GetVehicleState",
                GATEWAY_URL,
                r#"query GetVehicleState($vehicleID: String!) { vehicleState(id: $vehicleID) { powerState { timeStamp value } gnssLocation { latitude longitude timeStamp } batteryLevel { timeStamp value } distanceToEmpty { timeStamp value } vehicleMileage { timeStamp value } chargerState { timeStamp value } } }"#,
                Some("vehicleID"),
            ),
        };
        ReadRequest {
            operation,
            url,
            query,
            variables: variable.map_or(
                serde_json::Value::Null,
                |key| serde_json::json!({key: vehicle_id}),
            ),
        }
    }

    pub async fn execute<T: for<'de> Deserialize<'de>>(
        self,
        client: &reqwest::Client,
        tokens: &RivianTokenBundle,
        vehicle_id: &str,
    ) -> Result<T> {
        let request = self.request(vehicle_id);
        gql_request(
            client,
            request.url,
            tokens,
            request.operation,
            request.query,
            request.variables,
        )
        .await
    }
}

#[cfg(test)]
pub(super) const LIVE_HISTORY_TEST_QUERY: &str = r#"
    query getLiveSessionHistory($vehicleId: ID!) {
      getLiveSessionHistory(vehicleId: $vehicleId) {
        chartData { kw time }
      }
    }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_contract_catalog_retains_every_selection_origin_and_variable() {
        let frozen: serde_json::Value =
            serde_json::from_str(include_str!("request_contracts.json")).unwrap();
        for (operation, name, url, variable) in [
            (
                ReadOperation::Baseline,
                "GetVehicleState",
                GATEWAY_URL,
                Some("vehicleID"),
            ),
            (ReadOperation::Enrichment, "getUserInfo", GATEWAY_URL, None),
            (
                ReadOperation::Wallboxes,
                "getRegisteredWallboxes",
                CHRG_URL,
                None,
            ),
            (
                ReadOperation::CompletedHistory,
                "getCompletedSessionSummaries",
                CHRG_URL,
                None,
            ),
            (
                ReadOperation::LiveHistory,
                "getLiveSessionHistory",
                CHRG_URL,
                Some("vehicleId"),
            ),
            (
                ReadOperation::ChargingSchedule,
                "GetChargingSchedule",
                GATEWAY_URL,
                Some("vehicleId"),
            ),
            (
                ReadOperation::DepartureSchedules,
                "getDepartureSchedules",
                GATEWAY_URL,
                Some("vehicleId"),
            ),
            (
                ReadOperation::OtaDetails,
                "getOTAUpdateDetails",
                GATEWAY_URL,
                Some("vehicleId"),
            ),
        ] {
            let request = operation.request("fixture-vehicle");
            assert_eq!(request.operation, name);
            assert_eq!(request.url, url);
            assert_eq!(request.query, frozen[name].as_str().unwrap());
            assert_eq!(
                request.variables,
                variable.map_or(
                    serde_json::Value::Null,
                    |key| serde_json::json!({key:"fixture-vehicle"})
                )
            );
            crate::services::outbound_policy::require_telemetry_query(request.query).unwrap();
        }
    }
}
