pub mod header;
pub mod reader;
pub mod region;
pub mod variant;

pub use header::Header;
pub use reader::{open_reader, InnerReader, Reader};
pub use variant::{
  get_format_tag_names, record_clear_info, record_format, record_info, record_sample,
  record_samples, record_set_info_flag, record_set_info_float, record_set_info_integer,
  record_set_info_string, record_to_string, FormatValue, InfoValue, Variant,
};
