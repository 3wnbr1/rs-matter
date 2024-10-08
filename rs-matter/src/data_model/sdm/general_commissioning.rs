/*
 *
 *    Copyright (c) 2020-2022 Project CHIP Authors
 *
 *    Licensed under the Apache License, Version 2.0 (the "License");
 *    you may not use this file except in compliance with the License.
 *    You may obtain a copy of the License at
 *
 *        http://www.apache.org/licenses/LICENSE-2.0
 *
 *    Unless required by applicable law or agreed to in writing, software
 *    distributed under the License is distributed on an "AS IS" BASIS,
 *    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 *    See the License for the specific language governing permissions and
 *    limitations under the License.
 */

use log::info;

use rs_matter_macros::idl_import;

use strum::{EnumDiscriminants, FromRepr};

use crate::data_model::objects::*;
use crate::tlv::{FromTLV, TLVElement, ToTLV, Utf8Str};
use crate::transport::exchange::Exchange;
use crate::transport::session::SessionMode;
use crate::{attribute_enum, cmd_enter};
use crate::{command_enum, error::*};

idl_import!(clusters = ["GeneralCommissioning"]);

pub use general_commissioning::Commands;
pub use general_commissioning::CommissioningErrorEnum;
pub use general_commissioning::RegulatoryLocationTypeEnum;
pub use general_commissioning::ID;

#[derive(FromRepr, EnumDiscriminants)]
#[repr(u16)]
pub enum Attributes {
    BreadCrumb(AttrType<u64>) = 0,
    BasicCommissioningInfo(()) = 1,
    RegConfig(AttrType<u8>) = 2,
    LocationCapability(AttrType<u8>) = 3,
    SupportsConcurrentConnection(AttrType<bool>) = 4,
}

attribute_enum!(Attributes);

command_enum!(Commands);

#[repr(u16)]
pub enum RespCommands {
    ArmFailsafeResp = 0x01,
    SetRegulatoryConfigResp = 0x03,
    CommissioningCompleteResp = 0x05,
}

#[derive(FromTLV, ToTLV)]
#[tlvargs(lifetime = "'a")]
struct CommonResponse<'a> {
    error_code: u8,
    debug_txt: Utf8Str<'a>,
}

pub const CLUSTER: Cluster<'static> = Cluster {
    id: ID as _,
    feature_map: 0,
    attributes: &[
        FEATURE_MAP,
        ATTRIBUTE_LIST,
        Attribute::new(
            AttributesDiscriminants::BreadCrumb as u16,
            Access::READ.union(Access::WRITE).union(Access::NEED_ADMIN),
            Quality::NONE,
        ),
        Attribute::new(
            AttributesDiscriminants::RegConfig as u16,
            Access::RV,
            Quality::NONE,
        ),
        Attribute::new(
            AttributesDiscriminants::LocationCapability as u16,
            Access::RV,
            Quality::FIXED,
        ),
        Attribute::new(
            AttributesDiscriminants::BasicCommissioningInfo as u16,
            Access::RV,
            Quality::FIXED,
        ),
        Attribute::new(
            AttributesDiscriminants::SupportsConcurrentConnection as u16,
            Access::RV,
            Quality::FIXED,
        ),
    ],
    commands: &[
        Commands::ArmFailSafe as _,
        Commands::SetRegulatoryConfig as _,
        Commands::CommissioningComplete as _,
    ],
};

#[derive(FromTLV, ToTLV)]
struct FailSafeParams {
    expiry_len: u16,
    bread_crumb: u64,
}

#[derive(FromTLV, ToTLV, Debug, Clone)]
pub struct BasicCommissioningInfo {
    pub expiry_len: u16,
    pub max_cmltv_failsafe_secs: u16,
}

impl BasicCommissioningInfo {
    pub const fn new() -> Self {
        // TODO: Arch-Specific
        Self {
            expiry_len: 120,
            max_cmltv_failsafe_secs: 120,
        }
    }
}

impl Default for BasicCommissioningInfo {
    fn default() -> Self {
        BasicCommissioningInfo::new()
    }
}

#[derive(Debug, Clone)]
pub struct GenCommCluster {
    data_ver: Dataver,
    basic_comm_info: BasicCommissioningInfo,
    supports_concurrent_connection: bool,
}

impl GenCommCluster {
    pub const fn new(
        data_ver: Dataver,
        basic_comm_info: BasicCommissioningInfo,
        supports_concurrent_connection: bool,
    ) -> Self {
        Self {
            data_ver,
            basic_comm_info,
            supports_concurrent_connection,
        }
    }

    pub fn read(
        &self,
        _exchange: &Exchange,
        attr: &AttrDetails,
        encoder: AttrDataEncoder,
    ) -> Result<(), Error> {
        if let Some(mut writer) = encoder.with_dataver(self.data_ver.get())? {
            if attr.is_system() {
                CLUSTER.read(attr.attr_id, writer)
            } else {
                match attr.attr_id.try_into()? {
                    Attributes::BreadCrumb(codec) => codec.encode(writer, 0),
                    // TODO: Arch-Specific
                    Attributes::RegConfig(codec) => {
                        codec.encode(writer, RegulatoryLocationTypeEnum::IndoorOutdoor as _)
                    }
                    // TODO: Arch-Specific
                    Attributes::LocationCapability(codec) => {
                        codec.encode(writer, RegulatoryLocationTypeEnum::IndoorOutdoor as _)
                    }
                    Attributes::BasicCommissioningInfo(_) => {
                        self.basic_comm_info
                            .to_tlv(&AttrDataWriter::TAG, &mut *writer)?;
                        writer.complete()
                    }
                    Attributes::SupportsConcurrentConnection(codec) => {
                        codec.encode(writer, self.supports_concurrent_connection)
                    }
                }
            }
        } else {
            Ok(())
        }
    }

    pub fn invoke(
        &self,
        exchange: &Exchange,
        cmd: &CmdDetails,
        data: &TLVElement,
        encoder: CmdDataEncoder,
    ) -> Result<(), Error> {
        match cmd.cmd_id.try_into()? {
            Commands::ArmFailSafe => self.handle_command_armfailsafe(exchange, data, encoder)?,
            Commands::SetRegulatoryConfig => {
                self.handle_command_setregulatoryconfig(exchange, data, encoder)?
            }
            Commands::CommissioningComplete => {
                self.handle_command_commissioningcomplete(exchange, encoder)?;
            }
        }

        self.data_ver.changed();

        Ok(())
    }

    fn handle_command_armfailsafe(
        &self,
        exchange: &Exchange,
        data: &TLVElement,
        encoder: CmdDataEncoder,
    ) -> Result<(), Error> {
        cmd_enter!("ARM Fail Safe");

        let p = FailSafeParams::from_tlv(data)?;

        let status = if exchange
            .matter()
            .failsafe
            .borrow_mut()
            .arm(
                p.expiry_len,
                exchange.with_session(|sess| Ok(sess.get_session_mode().clone()))?,
            )
            .is_err()
        {
            CommissioningErrorEnum::BusyWithOtherAdmin as u8
        } else {
            CommissioningErrorEnum::OK as u8
        };

        let cmd_data = CommonResponse {
            error_code: status,
            debug_txt: "",
        };

        encoder
            .with_command(RespCommands::ArmFailsafeResp as _)?
            .set(cmd_data)?;

        Ok(())
    }

    fn handle_command_setregulatoryconfig(
        &self,
        _exchange: &Exchange,
        data: &TLVElement,
        encoder: CmdDataEncoder,
    ) -> Result<(), Error> {
        cmd_enter!("Set Regulatory Config");
        let country_code = data
            .r#struct()
            .map_err(|_| ErrorCode::InvalidCommand)?
            .find_ctx(1)
            .map_err(|_| ErrorCode::InvalidCommand)?
            .utf8()
            .map_err(|_| ErrorCode::InvalidCommand)?;
        info!("Received country code: {}", country_code);

        let cmd_data = CommonResponse {
            error_code: 0,
            debug_txt: "",
        };

        encoder
            .with_command(RespCommands::SetRegulatoryConfigResp as _)?
            .set(cmd_data)?;

        Ok(())
    }

    fn handle_command_commissioningcomplete(
        &self,
        exchange: &Exchange,
        encoder: CmdDataEncoder,
    ) -> Result<(), Error> {
        cmd_enter!("Commissioning Complete");
        let mut status: u8 = CommissioningErrorEnum::OK as u8;

        // Has to be a Case Session
        if !exchange
            .with_session(|sess| Ok(matches!(sess.get_session_mode(), SessionMode::Case { .. })))?
        {
            status = CommissioningErrorEnum::InvalidAuthentication as u8;
        }

        // AddNOC or UpdateNOC must have happened, and that too for the same fabric
        // scope that is for this session
        if exchange
            .matter()
            .failsafe
            .borrow_mut()
            .disarm(exchange.with_session(|sess| Ok(sess.get_session_mode().clone()))?)
            .is_err()
        {
            status = CommissioningErrorEnum::InvalidAuthentication as u8;
        }

        let cmd_data = CommonResponse {
            error_code: status,
            debug_txt: "",
        };

        encoder
            .with_command(RespCommands::CommissioningCompleteResp as _)?
            .set(cmd_data)?;

        Ok(())
    }
}

impl Handler for GenCommCluster {
    fn read(
        &self,
        exchange: &Exchange,
        attr: &AttrDetails,
        encoder: AttrDataEncoder,
    ) -> Result<(), Error> {
        GenCommCluster::read(self, exchange, attr, encoder)
    }

    fn invoke(
        &self,
        exchange: &Exchange,
        cmd: &CmdDetails,
        data: &TLVElement,
        encoder: CmdDataEncoder,
    ) -> Result<(), Error> {
        GenCommCluster::invoke(self, exchange, cmd, data, encoder)
    }
}

impl NonBlockingHandler for GenCommCluster {}

impl ChangeNotifier<()> for GenCommCluster {
    fn consume_change(&mut self) -> Option<()> {
        self.data_ver.consume_change(())
    }
}
