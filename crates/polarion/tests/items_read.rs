//! Work item reads beyond get/query, and schema reads.

mod common;

use common::*;
use polarion::{Fields, ItemRef, WorkItemQuery};
use serde_json::json;

#[tokio::test]
async fn history_and_workflow_actions() {
    let server = server().await;
    let item = ItemRef::new("P", "P-1");
    expect(
        &server,
        TRACKER,
        &["generateHistory>", &format!("workitemURI>{}<", item.uri)],
        r#"<generateHistoryResponse xmlns:t="http://ws.polarion.com/TrackerWebService-types">
<generateHistoryReturn><creation>true</creation><date>2024-01-01T10:00:00.000Z</date><empty>false</empty><invalid>false</invalid><revision>100</revision><user>jdoe</user></generateHistoryReturn>
<generateHistoryReturn><creation>false</creation><date>2024-01-02T10:00:00.000Z</date>
  <diffs>
    <item><after xsi:type="t:EnumOptionId"><t:id>open</t:id></after><before xsi:type="t:EnumOptionId"><t:id>draft</t:id></before><collection>false</collection><fieldName>status</fieldName></item>
    <item><added><item xsi:type="xsd:string">amy</item></added><collection>true</collection><fieldName>assignee</fieldName></item>
  </diffs>
  <empty>false</empty><invalid>false</invalid><revision>101</revision><user>amy</user></generateHistoryReturn>
</generateHistoryResponse>"#,
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["getUnavailableActions>"],
        r#"<getUnavailableActionsResponse><getUnavailableActionsReturn><actionId>5</actionId><actionName>Close</actionName><targetStatus><id>done</id></targetStatus><requiredFeatures><item>resolution</item></requiredFeatures><unavailabilityMessage>Resolution is required</unavailabilityMessage></getUnavailableActionsReturn></getUnavailableActionsResponse>"#,
    )
    .await;

    let polarion = reader(&server);
    let history = polarion.items().history(&item).await.unwrap();
    assert_eq!(
        serde_json::to_value(&history).unwrap(),
        json!([
            {"revision": "100", "date": "2024-01-01T10:00:00.000Z", "user": "jdoe", "creation": true},
            {"revision": "101", "date": "2024-01-02T10:00:00.000Z", "user": "amy", "changes": [
                {"field": "status", "before": "draft", "after": "open"},
                {"field": "assignee", "added": ["amy"]},
            ]},
        ])
    );

    let unavailable = polarion.items().unavailable_actions(&item).await.unwrap();
    assert_eq!(
        serde_json::to_value(&unavailable).unwrap(),
        json!([{
            "id": 5, "name": "Close", "targetStatus": "done",
            "requiredFields": ["resolution"], "message": "Resolution is required",
        }])
    );
}

#[tokio::test]
async fn schema_reads_enum_ids_from_the_subtype() {
    let server = server().await;
    expect(
        &server,
        TRACKER,
        &["getDefinedCustomFieldTypes>", "projectID>P<", "typeID>requirement<"],
        r#"<getDefinedCustomFieldTypesResponse xmlns:t="http://ws.polarion.com/TrackerWebService-types">
<getDefinedCustomFieldTypesReturn xsi:type="t:EnumCustomFieldType" xmlns:q="http://ws.polarion.com/TrackerWebService-types"><t:defaultValue xsi:nil="true"/><t:id>asil</t:id><t:multi>false</t:multi><t:name>ASIL</t:name><t:required>true</t:required><t:type>q:EnumOptionId</t:type><t:enumId>asil-levels</t:enumId></getDefinedCustomFieldTypesReturn>
<getDefinedCustomFieldTypesReturn xmlns:x="http://www.w3.org/2001/XMLSchema"><t:id>reviewed</t:id><t:multi>false</t:multi><t:required>false</t:required><t:type>x:date</t:type></getDefinedCustomFieldTypesReturn>
</getDefinedCustomFieldTypesResponse>"#,
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["getAllEnumOptionsForKey>", "key>status<"],
        r#"<getAllEnumOptionsForKeyResponse><getAllEnumOptionsForKeyReturn><default>true</default><enumId>status</enumId><hidden>false</hidden><id>draft</id><name>Draft</name><phantom>false</phantom><properties><property><key>color</key><value>#ccc</value></property></properties><sequenceNumber>0</sequenceNumber></getAllEnumOptionsForKeyReturn></getAllEnumOptionsForKeyResponse>"#,
    )
    .await;

    let polarion = reader(&server);
    let fields = polarion
        .schema()
        .custom_fields("P", "requirement")
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&fields).unwrap(),
        json!([
            {"id": "asil", "name": "ASIL", "type": "EnumOptionId", "required": true, "enumId": "asil-levels"},
            {"id": "reviewed", "type": "date"},
        ])
    );

    let options = polarion
        .schema()
        .field_options("P", "status")
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&options).unwrap(),
        json!([{"id": "draft", "name": "Draft", "default": true, "sequence": 0, "properties": {"color": "#ccc"}}])
    );
}

#[tokio::test]
async fn query_variants() {
    let server = server().await;
    expect(
        &server,
        TRACKER,
        &[
            "queryWorkItemsInBaselineLimited>",
            "query>project.id:P AND (type:task)<",
            "baselineRevision>1234<",
            "resultsLimit>10<",
        ],
        "<queryWorkItemsInBaselineLimitedResponse><queryWorkItemsInBaselineLimitedReturn><id>P-1</id></queryWorkItemsInBaselineLimitedReturn></queryWorkItemsInBaselineLimitedResponse>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["queryWorkItemUrisLimited>", "query>type:task<", "resultsLimit>-1<"],
        &format!(
            "<queryWorkItemUrisLimitedResponse><queryWorkItemUrisLimitedReturn>{URI}P${{WorkItem}}P-1</queryWorkItemUrisLimitedReturn><queryWorkItemUrisLimitedReturn>junk</queryWorkItemUrisLimitedReturn></queryWorkItemUrisLimitedResponse>"
        ),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["queryWorkItemsBySQL>", "sqlQuery>SELECT * FROM WORKITEM<", "fields>id<"],
        "<queryWorkItemsBySQLResponse><queryWorkItemsBySQLReturn><id>P-2</id></queryWorkItemsBySQLReturn></queryWorkItemsBySQLResponse>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["getWorkItemByUriWithFields>", "uri>junk<", "keys>title<"],
        "<getWorkItemByUriWithFieldsResponse><getWorkItemByUriWithFieldsReturn uri=\"junk\"><title>T</title></getWorkItemByUriWithFieldsReturn></getWorkItemByUriWithFieldsResponse>",
    )
    .await;

    let polarion = reader(&server);
    let items = polarion.items();
    let in_baseline = items
        .query(
            &WorkItemQuery::new("type:task")
                .project("P")
                .limit(10)
                .baseline("1234"),
        )
        .await
        .unwrap();
    assert_eq!(in_baseline[0].id.as_deref(), Some("P-1"));

    let refs = items
        .query_refs(&WorkItemQuery::new("type:task").unlimited())
        .await
        .unwrap();
    assert_eq!(refs[0], ItemRef::new("P", "P-1"));
    assert_eq!(
        (refs[1].id.as_deref(), refs[1].uri.as_str()),
        (None, "junk")
    );

    let by_sql = items
        .query_sql("SELECT * FROM WORKITEM", &Fields::new(["id"]))
        .await
        .unwrap();
    assert_eq!(by_sql[0].id.as_deref(), Some("P-2"));

    // An unparsable reference is read by URI.
    let item = items
        .get_fields(&refs[1], &Fields::new(["title"]))
        .await
        .unwrap();
    assert_eq!(item.title.as_deref(), Some("T"));
}
