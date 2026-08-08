//! 知识检索模块：加载知识库、向量/关键词检索、FAQ 命中、答案缓存、运行时联网兜底。
mod cache;
mod faq;
mod knowledge;
mod rag;
mod web_search;

pub use cache::AnswerCache;
pub use faq::FaqStore;
pub use knowledge::{load, retrieve};
pub use rag::retrieve_scored;
pub use web_search::fetch_zero_labs_context;
