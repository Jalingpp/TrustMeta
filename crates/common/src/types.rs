use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub type Fid = String;
pub type Keyword = String;
pub type RootHash = Vec<u8>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proof {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdsMode {
    Mpt,
    Mest,
    AccTrie,
    AccTree,
}

impl AdsMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdsMode::Mpt => "mpt",
            AdsMode::Mest => "mest",
            AdsMode::AccTrie => "acctrie",
            AdsMode::AccTree => "acctree",
        }
    }
}

impl fmt::Display for AdsMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AdsMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "mpt" => Ok(AdsMode::Mpt),
            "mest" => Ok(AdsMode::Mest),
            "acctrie" | "accumulator" => Ok(AdsMode::AccTrie),
            "acctree" => Ok(AdsMode::AccTree),
            other => Err(format!(
                "Unknown ADS mode: {}. Expected mpt|mest|acctrie|acctree",
                other
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SetProofMode {
    Polynomial,
    Accumulator,
}

impl SetProofMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SetProofMode::Polynomial => "polynomial",
            SetProofMode::Accumulator => "accumulator",
        }
    }
}

impl fmt::Display for SetProofMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SetProofMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "polynomial" | "poly" => Ok(SetProofMode::Polynomial),
            "accumulator" | "acc" => Ok(SetProofMode::Accumulator),
            other => Err(format!(
                "Unknown set proof mode: {}. Expected polynomial|accumulator",
                other
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfig {
    pub num_clients: usize,
    pub num_storagers: usize,
    pub ads_mode: AdsMode,
    pub set_proof_mode: SetProofMode,
    pub manager_addr: String,
    pub storager_addrs: Vec<String>,
    pub client_addrs: Vec<String>,
}
