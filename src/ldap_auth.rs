use std::ptr;

use log::{debug, info, warn};
use failure::format_err;
use openldap::{self, LDAPResponse, RustLDAP};
use regex::Regex;

use crate::config::LdapConfig;
use crate::errors::*;

pub struct LDAPEntry {
    pub dn: String,
}

fn new_ldap(url: &str) -> Result<RustLDAP> {
    let ldap = RustLDAP::new(url)?;

    ldap.set_option(
        openldap::codes::options::LDAP_OPT_PROTOCOL_VERSION,
        &openldap::codes::versions::LDAP_VERSION3,
    );

    ldap.set_option(
        openldap::codes::options::LDAP_OPT_X_TLS_REQUIRE_CERT,
        &openldap::codes::options::LDAP_OPT_X_TLS_DEMAND,
    );

    Ok(ldap)
}

pub fn auth(user: &str, pass: &str, config: &LdapConfig) -> Result<bool> {
    if user.is_empty() {
        info!("Cannot authenticate without username");
        return Ok(false);
    }

    // in the absence of `ldap_escape` from ldap3, just whitelist acceptable characters
    let re = Regex::new(r"([^A-Za-z0-9\.\-_@])").unwrap();
    for cap in re.captures_iter(user) {
        info!("Invalid username character in username: '{}', '{}'", &cap[1], user);
        return Ok(false);
    }

    let user_filters = config.userid_attributes.iter().map(|a| format!("({}={})", a, user)).collect::<Vec<_>>();

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
    let ldap = new_ldap(&config.url)?;
    let res = ldap.simple_bind(&user_dn, &pass)?;
    if res == 0 {
        Ok(true)
    } else if res == 49 {
        // Avoid error messages for invalid creds
        Ok(false)
    } else {
        info!("LDAP auth failed with error code {}", res);
        Ok(false)
    }
}

pub fn search(config: &LdapConfig, extra_filter: Option<&str>, max_results: i32) -> Result<Vec<LDAPEntry>> {
    let ldap = new_ldap(&config.url)?;
    let bind_res = ldap.simple_bind(&config.bind_user, &config.bind_pass)?;
    if bind_res != 0 {
        return Err(format_err!("LDAP service account bind failed with error code {}", bind_res));
    }

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


    let resp: Result<LDAPResponse> = ldap.ldap_search(
        &config.base_dn,
        openldap::codes::scopes::LDAP_SCOPE_SUB,
        Some(&search_filter),
        None, // attrs
        false, // attrsonly
        None, // server controls
        None, // client controls
        ptr::null_mut(), // timeout
        max_results,
    ).map_err(|e| format_err!("Error on LDAP search: {}", e));

    let entries = resp?.into_iter()
        .filter_map(|attrs| {
            let dn = attrs.get("dn").unwrap_or(&vec![]).iter().next().map(|s| s.to_string()).unwrap_or(
                String::new(),
            );
            if dn.is_empty() {
                warn!("Found entry with empty DN! Skipping.");
                None
            } else {
                Some(LDAPEntry { dn: dn })
            }
        })
        .collect::<Vec<LDAPEntry>>();

    Ok(entries)
}
