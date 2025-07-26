utils::configurable_constants! {

    // Approximately 4 MB min spacing between global dedup queries.  Calculated by 4MB / TARGET_CHUNK_SIZE
    ref MIN_SPACING_BETWEEN_GLOBAL_DEDUP_QUERIES: usize = 256;

    /// scheme for a local filesystem based CAS server
    ref LOCAL_CAS_SCHEME: String = "local://".to_owned();

    /// The current version
    ref CURRENT_VERSION: String = release_fixed( env!("CARGO_PKG_VERSION").to_owned());

    /// The expiration time of a local shard when first placed in the local shard cache.  Currently
    /// set to 3 weeks.
    ref MDB_SHARD_LOCAL_CACHE_EXPIRATION_SECS: u64 = 3 * 7 * 24 * 3600;

    /// The maximum number of simultaneous xorb upload streams.
    /// The default value is 8 and can be overwritten by environment variable "XET_CONCURRENT_XORB_UPLOADS".
    ref MAX_CONCURRENT_UPLOADS: usize = 8;

    /// The maximum number of files to ingest at once on the upload path
    ref MAX_CONCURRENT_FILE_INGESTION: usize = 8;

    /// The maximum number of files to download at one time.
    ref MAX_CONCURRENT_DOWNLOADS : usize = 8;

    /// The maximum block size from a file to process at once.
    ref INGESTION_BLOCK_SIZE : usize = 8 * 1024 * 1024;

}
