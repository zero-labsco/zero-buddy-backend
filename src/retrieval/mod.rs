//! 知识检索模块：加载知识库、向量/关键词检索、FAQ 命中、答案缓存。
mod cache;
mod faq;
mod knowledge;
mod rag;

pub use cache::AnswerCache;
pub use faq::FaqStore;
pub use knowledge::{load, retrieve};
pub use rag::retrieve_scored;
