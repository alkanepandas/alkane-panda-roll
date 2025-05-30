use metashrew_support::index_pointer::KeyValuePointer;
use metashrew_support::compat::to_arraybuffer_layout;
use metashrew_support::block::AuxpowBlock;
use metashrew_support::utils::consensus_decode;

use alkanes_runtime::{
  declare_alkane, message::MessageDispatch, storage::StoragePointer, token::Token,
  runtime::AlkaneResponder
};

use alkanes_support::{
  id::AlkaneId,
  parcel::AlkaneTransfer, response::CallResponse
};

use bitcoin::hashes::Hash;
use bitcoin::{Txid, Block, Transaction};

use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::io::Cursor;

// We could validate pandas ids against the collection contract 2:614, but we cbf. Save fuel.
mod panda_ids;
use panda_ids::PANDA_IDS;

const PANDA_BLOCK: u128 = 0x2;

#[derive(Default)]
pub struct PandaRoll(());

impl AlkaneResponder for PandaRoll {}

#[derive(MessageDispatch)]
enum PandaRollMessage {
  #[opcode(0)]
  Initialize,

  #[opcode(42)]
  Deposit,

  #[opcode(69)]
  Roll,

  #[opcode(99)]
  #[returns(String)]
  GetName,

  #[opcode(100)]
  #[returns(String)]
  GetSymbol,

  #[opcode(101)]
  #[returns(u128)]
  GetPandaStackCount,

  #[opcode(102)]
  #[returns(Vec<Vec<u8>>)]
  GetPandaStack,

  #[opcode(103)]
  #[returns(String)]
  GetPandaStackJson,
}

impl Token for PandaRoll {
  fn name(&self) -> String {
    return String::from("Alkane Panda Roll")
  }

  fn symbol(&self) -> String {
    return String::from("alkane-panda-roll");
  }
}

impl PandaRoll {
  fn initialize(&self) -> Result<CallResponse> {
    self.observe_initialization()?;
    let context = self.context()?;

    let response = CallResponse::forward(&context.incoming_alkanes);
    Ok(response)
  }

  fn get_name(&self) -> Result<CallResponse> {
    let context = self.context()?;
    let mut response = CallResponse::forward(&context.incoming_alkanes);

    response.data = self.name().into_bytes();

    Ok(response)
  }

  fn get_symbol(&self) -> Result<CallResponse> {
    let context = self.context()?;
    let mut response = CallResponse::forward(&context.incoming_alkanes);

    response.data = self.symbol().into_bytes();

    Ok(response)
  }

  fn is_valid_panda(&self, id: &AlkaneId) -> Result<bool> {
    Ok(id.block == PANDA_BLOCK && PANDA_IDS.contains(&id.tx))
  }

  fn deposit(&self) -> Result<CallResponse> {
    let context = self.context()?;

    for alkane in context.incoming_alkanes.0.iter() {
      if !self.is_valid_panda(&alkane.id)? {
        return Err(anyhow!("Invalid Panda ID"));
      }

      self.add_instance(&alkane.id)?;
    }

    Ok(CallResponse::default())
  }

  fn roll(&self) -> Result<CallResponse> {
    let context = self.context()?;
    let mut response = CallResponse::forward(&context.incoming_alkanes);

    let txid = self.transaction_id()?;

    // Enforce one roll per transaction
    if self.has_tx_hash(&txid) {
      return Err(anyhow!("Transaction already used for roll"));
    }
    
    if context.incoming_alkanes.0.len() != 1 {
      return Err(anyhow!("Must send 1 Panda to roll"));
    }

    if !self.is_valid_panda(&context.incoming_alkanes.0[0].id)? {
      return Err(anyhow!("Invalid Panda ID"));
    }

    let count = self.instances_count();
    if count < 1 {
      return Err(anyhow!("Not enough Pandas available to roll"));
    }

    self.add_tx_hash(&txid)?;

    let multiplier = self.calculate_random_multiplier()?;

    if multiplier == 0 {
      self.add_instance(&context.incoming_alkanes.0[0].id)?;
      return Ok(CallResponse::default());
    }

    // Win case - add one more panda
    let instance_id = self.pop_instance()?;
    response.alkanes.0.push(AlkaneTransfer {
      id: instance_id,
      value: 1u128,
    });

    Ok(response)
  }

  fn calculate_random_multiplier(&self) -> Result<u8> {
    let block_hash = self.block_hash()?;
    let txid = self.transaction_id()?;
    let txid_bytes = txid.as_byte_array();
  
    let value = block_hash[31].wrapping_add(txid_bytes[31]);
  
    Ok(if value < 141 { 0 } else { 2 })
  }
  
  fn instances_pointer(&self) -> StoragePointer {
    StoragePointer::from_keyword("/instances")
  }

  fn instances_count(&self) -> u128 {
    self.instances_pointer().get_value::<u128>()
  }

  fn set_instances_count(&self, count: u128) {
    self.instances_pointer().set_value::<u128>(count);
  }

  fn add_instance(&self, instance_id: &AlkaneId) -> Result<u128> {
    let count = self.instances_count();
    let new_count = count.checked_add(1)
      .ok_or_else(|| anyhow!("instances count overflow"))?;

    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(&instance_id.block.to_le_bytes());
    bytes.extend_from_slice(&instance_id.tx.to_le_bytes());

    let bytes_vec = new_count.to_le_bytes().to_vec();
    let mut instance_pointer = self.instances_pointer().select(&bytes_vec);
    instance_pointer.set(Arc::new(bytes));
    
    self.set_instances_count(new_count);
    
    Ok(new_count)
  }

  fn pop_instance(&self) -> Result<AlkaneId> {
    let count = self.instances_count();

    let new_count = count.checked_sub(1)
      .ok_or_else(|| anyhow!("instances count underflow"))?;

    let instance_id = self.lookup_instance(count - 1)?;
    
    // Remove the instance by setting it to empty
    let bytes_vec = count.to_le_bytes().to_vec();
    let mut instance_pointer = self.instances_pointer().select(&bytes_vec);
    instance_pointer.set(Arc::new(Vec::new()));
    
    self.set_instances_count(new_count);
    
    Ok(instance_id)
  }

  fn lookup_instance(&self, index: u128) -> Result<AlkaneId> {
    let bytes_vec = (index + 1).to_le_bytes().to_vec();
    let instance_pointer = self.instances_pointer().select(&bytes_vec);
    
    let bytes = instance_pointer.get();
    if bytes.len() != 32 {
      return Err(anyhow!("Invalid instance data length"));
    }

    let block_bytes = &bytes[..16];
    let tx_bytes = &bytes[16..];

    let block = u128::from_le_bytes(block_bytes.try_into().unwrap());
    let tx = u128::from_le_bytes(tx_bytes.try_into().unwrap());

    Ok(AlkaneId { block, tx })
  }

  fn get_panda_stack_count(&self) -> Result<CallResponse> {
    let context = self.context()?;
    let mut response = CallResponse::forward(&context.incoming_alkanes);

    response.data = self.instances_count().to_le_bytes().to_vec();

    Ok(response)
  }

  fn get_panda_stack(&self) -> Result<CallResponse> {
    let context = self.context()?;
    let mut response = CallResponse::forward(&context.incoming_alkanes);

    let count = self.instances_count();
    let mut panda_ids = Vec::new();

    for i in 0..count {
      let instance_id = self.lookup_instance(i)?;
      let mut bytes = Vec::with_capacity(32);
      bytes.extend_from_slice(&instance_id.block.to_le_bytes());
      bytes.extend_from_slice(&instance_id.tx.to_le_bytes());
      panda_ids.push(bytes);
    }

    let mut flattened = Vec::new();
    for bytes in panda_ids {
      flattened.extend(bytes);
    }

    response.data = flattened;
    Ok(response)
  }

  fn get_panda_stack_json(&self) -> Result<CallResponse> {
    let context = self.context()?;
    let mut response = CallResponse::forward(&context.incoming_alkanes);

    let count = self.instances_count();
    let mut panda_ids = Vec::new();

    for i in 0..count {
      let instance_id = self.lookup_instance(i)?;
      panda_ids.push(format!("{}:{}", instance_id.block, instance_id.tx));
    }

    response.data = serde_json::to_string(&panda_ids)?.into_bytes();
    Ok(response)
  }

  fn current_block(&self) -> Result<Block> {
    Ok(AuxpowBlock::parse(&mut Cursor::<Vec<u8>>::new(self.block()))?.to_consensus())
  }

  fn block_hash(&self) -> Result<Vec<u8>> {
    let hash = self.current_block()?.block_hash().as_byte_array().to_vec();
    Ok(hash)
  }

  fn transaction_id(&self) -> Result<Txid> {
    Ok(
      consensus_decode::<Transaction>(&mut std::io::Cursor::new(self.transaction()))?
        .compute_txid(),
    )
  }

  fn has_tx_hash(&self, txid: &Txid) -> bool {
    StoragePointer::from_keyword("/tx-hashes/")
      .select(&txid.as_byte_array().to_vec())
      .get_value::<u8>()
      == 1
  }

  fn add_tx_hash(&self, txid: &Txid) -> Result<()> {
    StoragePointer::from_keyword("/tx-hashes/")
      .select(&txid.as_byte_array().to_vec())
      .set_value::<u8>(0x01);

    Ok(())
  }
}

declare_alkane! {
  impl AlkaneResponder for PandaRoll {
    type Message = PandaRollMessage;
  }
}
