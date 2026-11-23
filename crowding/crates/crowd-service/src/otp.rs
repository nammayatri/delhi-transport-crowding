//! OpenTripPlanner 2.5 GTFS GraphQL client (`plan` query).

use serde::Deserialize;
use serde_json::json;

use crate::config::OtpConfig;
use crate::error::{AppError, AppResult};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtpItinerary {
    pub start_time: i64,
    pub end_time: i64,
    pub legs: Vec<OtpLeg>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtpLeg {
    pub mode: String,
    pub start_time: i64,
    pub end_time: i64,
    pub from: OtpPlace,
    pub to: OtpPlace,
    pub route: Option<OtpRoute>,
    pub trip: Option<OtpTrip>,
    pub leg_geometry: Option<OtpGeometry>,
}

#[derive(Debug, Deserialize)]
pub struct OtpPlace {
    pub name: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub stop: Option<OtpStop>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtpStop {
    pub gtfs_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtpRoute {
    pub gtfs_id: String,
    pub short_name: Option<String>,
    pub long_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtpTrip {
    pub gtfs_id: String,
}

#[derive(Debug, Deserialize)]
pub struct OtpGeometry {
    pub points: String,
}

const QUERY: &str = r#"
query Plan($from: InputCoordinates!, $to: InputCoordinates!, $date: String!, $time: String!, $n: Int!__RELUCTANCE_VAR__) {
  plan(from: $from, to: $to, date: $date, time: $time, numItineraries: $n,
       transportModes: [{mode: BUS}, {mode: WALK}]__RELUCTANCE_ARG__) {
    itineraries {
      startTime endTime
      legs {
        mode startTime endTime
        from { name lat lon stop { gtfsId } }
        to { name lat lon stop { gtfsId } }
        route { gtfsId shortName longName }
        trip { gtfsId }
        legGeometry { points }
      }
    }
  }
}"#;

pub async fn plan(
    http: &reqwest::Client,
    cfg: &OtpConfig,
    from: (f64, f64),
    to: (f64, f64),
    date: &str,
    time: &str,
    reluctance: Option<f64>,
) -> AppResult<Vec<OtpItinerary>> {
    let with_rel = cfg.crowding_patch && reluctance.is_some();
    let q = if with_rel {
        QUERY
            .replace("__RELUCTANCE_VAR__", ", $rel: Float")
            .replace("__RELUCTANCE_ARG__", ", crowdingReluctance: $rel")
    } else {
        QUERY
            .replace("__RELUCTANCE_VAR__", "")
            .replace("__RELUCTANCE_ARG__", "")
    };
    let mut vars = json!({
        "from": {"lat": from.0, "lon": from.1}, "to": {"lat": to.0, "lon": to.1},
        "date": date, "time": time, "n": cfg.num_itineraries,
    });
    if with_rel {
        vars["rel"] = json!(reluctance.unwrap_or(0.0));
    }
    let resp = http
        .post(&cfg.graphql_url)
        .json(&json!({"query": q, "variables": vars}))
        .send()
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;
    if let Some(errs) = v.get("errors") {
        return Err(AppError::Upstream(errs.to_string()));
    }
    let its = v["data"]["plan"]["itineraries"].clone();
    serde_json::from_value(its).map_err(|e| AppError::Upstream(format!("bad OTP response: {e}")))
}

/// Decode a Google encoded polyline into [lon, lat] pairs.
pub fn decode_polyline(s: &str) -> Vec<[f64; 2]> {
    let b = s.as_bytes();
    let (mut i, mut lat, mut lon) = (0usize, 0i64, 0i64);
    let mut out = Vec::new();
    let next = |i: &mut usize| -> Option<i64> {
        let (mut shift, mut result) = (0, 0i64);
        loop {
            let c = *b.get(*i)? as i64 - 63;
            *i += 1;
            result |= (c & 0x1f) << shift;
            shift += 5;
            if c < 0x20 {
                break;
            }
        }
        Some(if result & 1 != 0 {
            !(result >> 1)
        } else {
            result >> 1
        })
    };
    while i < b.len() {
        let (Some(dlat), Some(dlon)) = (next(&mut i), next(&mut i)) else {
            break;
        };
        lat += dlat;
        lon += dlon;
        out.push([lon as f64 / 1e5, lat as f64 / 1e5]);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_reference_polyline() {
        let p = super::decode_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@");
        assert_eq!(p.len(), 3);
        assert!((p[0][1] - 38.5).abs() < 1e-9 && (p[0][0] + 120.2).abs() < 1e-9);
        assert!((p[2][1] - 43.252).abs() < 1e-9 && (p[2][0] + 126.453).abs() < 1e-9);
    }
}
