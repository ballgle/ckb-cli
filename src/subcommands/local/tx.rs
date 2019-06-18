use std::path::PathBuf;

use ckb_core::transaction::{CellInput, CellOutput, OutPoint, TransactionBuilder, Witness};
use ckb_sdk::{
    with_rocksdb, CellInputManager, CellManager, HttpRpcClient, KeyManager, TransactionManager,
};
use clap::{App, Arg, ArgMatches, SubCommand};
use jsonrpc_types::TransactionView;
use numext_fixed_hash::H256;

use super::super::CliSubCommand;
use crate::utils::arg_parser::{ArgParser, FixedHashParser};
use crate::utils::printer::Printable;

pub struct LocalTxSubCommand<'a> {
    rpc_client: &'a mut HttpRpcClient,
    db_path: PathBuf,
}

impl<'a> LocalTxSubCommand<'a> {
    pub fn new(rpc_client: &'a mut HttpRpcClient, db_path: PathBuf) -> LocalTxSubCommand<'a> {
        LocalTxSubCommand {
            rpc_client,
            db_path,
        }
    }

    pub fn subcommand() -> App<'static, 'static> {
        let arg_tx_hash = Arg::with_name("tx-hash")
            .long("tx-hash")
            .takes_value(true)
            .validator(|input| FixedHashParser::<H256>::default().validate(input))
            .required(true)
            .help("Transaction hash");
        SubCommand::with_name("tx").subcommands(vec![
            SubCommand::with_name("add")
                .arg(
                    Arg::with_name("deps")
                        .long("deps")
                        .takes_value(true)
                        .multiple(true)
                        .help("Dependency cells"),
                )
                .arg(
                    Arg::with_name("inputs")
                        .long("inputs")
                        .takes_value(true)
                        .multiple(true)
                        .help("Input cells"),
                )
                .arg(
                    Arg::with_name("outputs")
                        .long("outputs")
                        .takes_value(true)
                        .multiple(true)
                        .help("Output cells"),
                )
                .arg(
                    Arg::with_name("set-witnesses-by-keys")
                        .long("set-witnesses-by-keys")
                        .help("Set input witnesses by saved private keys"),
                ),
            SubCommand::with_name("set-witness")
                .arg(arg_tx_hash.clone())
                .arg(
                    Arg::with_name("input")
                        .long("input")
                        .takes_value(true)
                        .required(true)
                        .help("Set witnesses for which input"),
                )
                .arg(
                    Arg::with_name("witness")
                        .long("witness")
                        .takes_value(true)
                        .multiple(true)
                        .help("Witness data list"),
                ),
            SubCommand::with_name("set-witnesses-by-keys").arg(arg_tx_hash.clone()),
            SubCommand::with_name("show").arg(arg_tx_hash.clone()),
            SubCommand::with_name("remove").arg(arg_tx_hash.clone()),
            SubCommand::with_name("verify").arg(arg_tx_hash.clone()),
            SubCommand::with_name("list"),
        ])
    }
}

impl<'a> CliSubCommand for LocalTxSubCommand<'a> {
    fn process(&mut self, matches: &ArgMatches) -> Result<Box<dyn Printable>, String> {
        match matches.subcommand() {
            ("add", Some(m)) => {
                let deps_result: Result<Vec<OutPoint>, String> = m
                    .values_of_lossy("deps")
                    .unwrap_or_else(Vec::new)
                    .into_iter()
                    .map(|dep_str| {
                        let parts = dep_str.split('-').collect::<Vec<_>>();
                        if parts.len() != 2 {
                            return Err(format!("Invalid deps: {}", dep_str));
                        }

                        let mut tx_hash_str =
                            *(parts.get(0).ok_or_else(|| "No tx hash found".to_owned())?);
                        if tx_hash_str.starts_with("0x") || tx_hash_str.starts_with("0X") {
                            tx_hash_str = &tx_hash_str[2..];
                        }
                        let tx_hash =
                            H256::from_hex_str(tx_hash_str).map_err(|err| err.to_string())?;
                        let index = parts
                            .get(1)
                            .ok_or_else(|| "No index found".to_owned())?
                            .parse::<u32>()
                            .map_err(|err| err.to_string())?;
                        Ok(OutPoint::new_cell(tx_hash, index))
                    })
                    .collect();
                let deps = deps_result?;
                let inputs_result: Result<Vec<CellInput>, String> = m
                    .values_of_lossy("inputs")
                    .unwrap_or_else(Vec::new)
                    .into_iter()
                    .map(|input_name| {
                        let input = with_rocksdb(&self.db_path, None, |db| {
                            CellInputManager::new(db)
                                .get(&input_name)
                                .map_err(Into::into)
                        })
                        .map_err(|err| format!("{:?}", err))?;
                        Ok(input)
                    })
                    .collect();
                let inputs = inputs_result?;
                let outputs_result: Result<Vec<CellOutput>, String> = m
                    .values_of_lossy("outputs")
                    .unwrap_or_else(Vec::new)
                    .into_iter()
                    .map(|output_name| {
                        let input = with_rocksdb(&self.db_path, None, |db| {
                            CellManager::new(db).get(&output_name).map_err(Into::into)
                        })
                        .map_err(|err| format!("{:?}", err))?;
                        Ok(input)
                    })
                    .collect();
                let outputs = outputs_result?;
                let set_witnesses_by_keys = m.is_present("set-witnesses-by-keys");

                let witnesses = inputs.iter().map(|_| Witness::new()).collect::<Vec<_>>();
                let mut tx = TransactionBuilder::default()
                    .deps(deps)
                    .inputs(inputs)
                    .outputs(outputs)
                    .witnesses(witnesses)
                    .build();
                with_rocksdb(&self.db_path, None, |db| {
                    TransactionManager::new(db).add(&tx).map_err(Into::into)
                })
                .map_err(|err| format!("{:?}", err))?;
                if set_witnesses_by_keys {
                    let db_path = self.db_path.clone();
                    tx = with_rocksdb(&db_path, None, |db| {
                        let keys = KeyManager::new(db).list()?;
                        TransactionManager::new(db)
                            .set_witnesses_by_keys(tx.hash(), &keys, self.rpc_client)
                            .map_err(Into::into)
                    })
                    .map_err(|err| format!("{:?}", err))?;
                }
                let tx_view: TransactionView = (&tx).into();
                Ok(Box::new(serde_json::to_string(&tx_view).unwrap()))
            }
            ("set-witness", Some(_m)) => Ok(Box::new("null".to_string())),
            ("set-witnesses-by-keys", Some(m)) => {
                let tx_hash_str = m.value_of("tx-hash").unwrap();
                let tx_hash = H256::from_hex_str(tx_hash_str).map_err(|err| err.to_string())?;
                let db_path = self.db_path.clone();
                let tx = with_rocksdb(&db_path, None, |db| {
                    let keys = KeyManager::new(db).list()?;
                    TransactionManager::new(db)
                        .set_witnesses_by_keys(&tx_hash, &keys, self.rpc_client)
                        .map_err(Into::into)
                })
                .map_err(|err| format!("{:?}", err))?;
                let tx_view: TransactionView = (&tx).into();
                Ok(Box::new(serde_json::to_string(&tx_view).unwrap()))
            }
            ("show", Some(m)) => {
                let tx_hash_str = m.value_of("tx-hash").unwrap();
                let tx_hash = H256::from_hex_str(tx_hash_str).map_err(|err| err.to_string())?;
                let tx = with_rocksdb(&self.db_path, None, |db| {
                    TransactionManager::new(db)
                        .get(&tx_hash)
                        .map_err(Into::into)
                })
                .map_err(|err| format!("{:?}", err))?;
                let tx_view: TransactionView = (&tx).into();
                Ok(Box::new(serde_json::to_string(&tx_view).unwrap()))
            }
            ("remove", Some(m)) => {
                let tx_hash: H256 =
                    FixedHashParser::<H256>::default().from_matches(m, "tx-hash")?;
                let tx = with_rocksdb(&self.db_path, None, |db| {
                    TransactionManager::new(db)
                        .remove(&tx_hash)
                        .map_err(Into::into)
                })
                .map_err(|err| format!("{:?}", err))?;
                let tx_view: TransactionView = (&tx).into();
                Ok(Box::new(serde_json::to_string(&tx_view).unwrap()))
            }
            ("verify", Some(m)) => {
                let tx_hash: H256 =
                    FixedHashParser::<H256>::default().from_matches(m, "tx-hash")?;
                let db_path = self.db_path.clone();
                let result = with_rocksdb(&db_path, None, |db| {
                    TransactionManager::new(db)
                        .verify(&tx_hash, std::u64::MAX, self.rpc_client)
                        .map_err(Into::into)
                })
                .map_err(|err| format!("{:?}", err))?;
                Ok(Box::new(serde_json::to_string(&result).unwrap()))
            }
            ("list", Some(_m)) => {
                let txs = with_rocksdb(&self.db_path, None, |db| {
                    TransactionManager::new(db).list().map_err(Into::into)
                })
                .map_err(|err| format!("{:?}", err))?;
                let txs = txs
                    .into_iter()
                    .map(|tx| {
                        let tx_view: TransactionView = (&tx).into();
                        serde_json::json!({
                            "tx": serde_json::to_value(&tx_view).unwrap(),
                            "tx-hash": tx.hash(),
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(Box::new(serde_json::to_string(&txs).unwrap()))
            }
            _ => Err(matches.usage().to_owned()),
        }
    }
}
