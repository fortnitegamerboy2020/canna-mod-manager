use super::*;
#[derive(Default, Deserialize)]
pub struct Page {
    pub page: Option<u32>,
    #[serde(default)]
    pub search: String,
}
impl Page {
    pub fn offset(&self) -> i64 {
        i64::from(self.page.unwrap_or(1).clamp(1, 1_000_000) - 1) * 50
    }
    pub fn limit(&self, legacy: i64) -> i64 {
        if self.page.is_some() { 50 } else { legacy }
    }
    pub fn term(&self) -> String {
        self.search.chars().take(100).collect()
    }
    pub fn response(&self, items: Vec<Value>, total: i64) -> Value {
        if self.page.is_some() {
            json!({"items":items,"total":total,"page":self.page.unwrap_or(1).clamp(1,1_000_000),"page_size":50})
        } else {
            json!(items)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn paginated_lists_find_members_beyond_legacy_caps() {
        let (_dir, app) = fixture();
        let token = account(&app, "Owner", true);
        app.db.lock().unwrap().execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<2500) INSERT INTO users(username,password,verified,role) SELECT printf('member%04d',x),'fixture',1,'member' FROM n;").unwrap();
        for path in ["profiles", "admin/users", "admin/wallets"] {
            let page = value(
                call(
                    app.clone(),
                    "GET",
                    &format!("/api/v1/{path}?page=21"),
                    Value::Null,
                    Some(&token),
                )
                .await,
            )
            .await;
            assert_eq!(page["total"], 2501);
            assert_eq!(page["items"].as_array().unwrap().len(), 50);
            let found = value(
                call(
                    app.clone(),
                    "GET",
                    &format!("/api/v1/{path}?page=1&search=member2500"),
                    Value::Null,
                    Some(&token),
                )
                .await,
            )
            .await;
            assert_eq!(found["total"], 1);
            assert_eq!(found["items"][0]["username"], "member2500");
            assert_eq!(
                call(
                    app.clone(),
                    "GET",
                    &format!("/api/v1/{path}?page=1"),
                    Value::Null,
                    None
                )
                .await
                .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        for path in ["admin/audit", "admin/tickets", "admin/mod-reviews"] {
            let page = value(
                call(
                    app.clone(),
                    "GET",
                    &format!("/api/v1/{path}?page=1&search=none"),
                    Value::Null,
                    Some(&token),
                )
                .await,
            )
            .await;
            assert_eq!(page["total"], 0);
            assert!(page["items"].as_array().unwrap().is_empty());
        }
        let member = account(&app, "Regular", false);
        assert_eq!(
            call(
                app,
                "GET",
                "/api/v1/admin/wallets?page=1",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
}
