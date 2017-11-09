use std::io;

use ldap3::{LdapConn, Scope, SearchEntry, ldap_escape, parse_filter};

use config::LdapConfig;

pub fn auth(user: &str, pass: &str, config: &LdapConfig) -> io::Result<bool> {
    if user.is_empty() {
        info!("Cannot authenticate without username");
        return Ok(false);
    }

    let user_safe = ldap_escape(user);
    let user_filters = config
        .userid_attributes
        .iter()
        .map(|a| format!("({}={})", a, user_safe.as_ref()))
        .collect::<Vec<_>>();

    let user_filter;
    if user_filters.len() == 0 {
        info!("Cannot authenticate without userid attributes");
        return Ok(false);
    } else if user_filters.len() == 1 {
        user_filter = user_filters[0].clone();
    } else {
        user_filter = format!("(|{})", user_filters.join(""));
    }

    // search for the user's DN
    let results = search(config, Some(&user_filter), 1)?;

    if results.is_empty() {
        debug!("No users found matching {}", user);
        return Ok(false);
    }
    if results.len() > 1 {
        info!("Too many users found matching {}", user);
        return Ok(false);
    }

    let user_dn = &results[0].dn;
    if user_dn.is_empty() {
        info!("User found but with empty DN!");
        return Ok(false);
    }

    // now try to bind as the user
    let ldap = LdapConn::new(&config.url)?;
    let res = ldap.simple_bind(&user_dn, &pass)?;
    if res.rc == 0 {
        ldap.unbind()?;
        Ok(true)
    } else if res.rc == 49 {
        // Avoid error messages for invalid creds
        Ok(false)
    } else {
        // should actually return an err
        res.success()?;
        Ok(false)
    }

}

pub fn search(config: &LdapConfig, extra_filter: Option<&str>, max_results: u32) -> io::Result<Vec<SearchEntry>> {
    let ldap = LdapConn::new(&config.url)?;
    ldap.simple_bind(&config.bind_user, &config.bind_pass)?.success()?;

    let mut search_filters: Vec<String> = vec![];
    if let Some(f) = extra_filter {
        search_filters.push(f.to_string());
    }
    if let Some(ref f) = config.search_filter {
        search_filters.push(f.to_string());
    }

    let search_filter;
    if search_filters.is_empty() {
        warn!("No LDAP search filter configured. There may be lots of results");
        search_filter = "(objectClass=*)".to_string();
    } else if search_filters.len() == 1 {
        search_filter = search_filters[0].clone();
    } else {
        search_filter = format!("(&{})", search_filters.join(""));
    }

    // `search` will panic prior to this commit:
    // https://github.com/inejge/ldap3/commit/25b99eea70e51d9a994d9c144191d5213da188bc
    if parse_filter(&search_filter).is_err() {
        return Err(io::Error::new(io::ErrorKind::Other, format!("Invalid search filter: {}", search_filter)));
    }

    let mut result = ldap.streaming_search(&config.base_dn, Scope::Subtree, &search_filter, vec!["*"])?;
    let mut results_found = 0;

    let mut entries: Vec<SearchEntry> = vec![];

    loop {
        match result.next() {
            Ok(Some(entry)) => {
                if results_found >= max_results {
                    result.abandon()?;
                    break;
                }
                entries.push(SearchEntry::construct(entry));
                results_found += 1;
            }
            Ok(None) => break,
            Err(e) => {
                error!("Error searching LDAP: {}", e);
                result.abandon()?;
            }
        };
    }

    result.result()?.success()?;

    // disconnect as the bind user
    ldap.unbind()?;

    Ok(entries)
}
