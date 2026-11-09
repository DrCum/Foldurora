//! Private staged upload phases. Each phase is released by a fresh native
//! assistant authority check. Uploader capabilities never enter public tools.
use super::*;
pub struct Offer {
    uploader: String,
    request: Request,
    thumbnail: Option<Uuid>,
}
impl Offer {
    pub fn cost(&self) -> i32 {
        self.request.expected_cost
    }
}
#[cfg(test)]
pub(crate) fn test_offer(request: Request, uploader: String) -> Offer {
    Offer {
        request,
        uploader,
        thumbnail: None,
    }
}
#[derive(Debug, Clone)]
pub struct Committed {
    pub item: Uuid,
    pub asset: Uuid,
    pub folder: Uuid,
    pub kind: Kind,
    pub cost: i32,
    pub cost_known: bool,
    pub approved_ceiling: i32,
    pub thumbnail: bool,
}
#[derive(Debug, Clone)]
pub enum ThumbnailReceipt {
    Item(Box<super::super::InvItem>),
    Folder(super::super::InvFolder),
}
pub async fn prepare(http: &reqwest::Client, cap: &str, request: Request) -> Result<Offer, Error> {
    if matches!(request.kind, Kind::Material | Kind::Mesh) {
        return Err(Error::Invalid);
    }
    let body = request.body()?;
    let response = http
        .post(cap)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(aurora_llsd::to_xml(&body))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| Error::Unavailable)?;
    let offer = reply(response).await?;
    if offer["state"].as_str() != "upload" || offer["uploader"].as_str().is_empty() {
        return Err(Error::Rejected);
    }
    if let Llsd::Integer(cost) = &offer["upload_price"]
        && (*cost < 0 || *cost > request.expected_cost)
    {
        return Err(if *cost < 0 {
            Error::Response
        } else {
            Error::PriceChanged(Some(*cost))
        });
    }
    Ok(Offer {
        uploader: offer["uploader"].as_str().to_owned(),
        request,
        thumbnail: None,
    })
}
pub async fn prepare_thumbnail(http: &reqwest::Client, cap: &str, item: Uuid, folder: bool, data: Vec<u8>) -> Result<Offer, Error> {
    if item.is_nil() || data.is_empty() || data.len() > 2 * 1024 * 1024 {
        return Err(Error::Invalid);
    }
    let body = if folder {
        llsd_map! {"category_id"=>item}
    } else {
        llsd_map! {"item_id"=>item}
    };
    let response = http
        .post(cap)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(aurora_llsd::to_xml(&body))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| Error::Unavailable)?;
    let offer = reply(response).await?;
    if offer["uploader"].as_str().is_empty() {
        return Err(Error::Rejected);
    }
    if !matches!(offer["upload_price"], Llsd::Undef | Llsd::Integer(_)) {
        return Err(Error::Response);
    }
    if let Llsd::Integer(cost) = &offer["upload_price"]
        && *cost != 0
    {
        return Err(if *cost > 0 {
            Error::PriceChanged(Some(*cost))
        } else {
            Error::Response
        });
    }
    Ok(Offer {
        uploader: offer["uploader"].as_str().to_owned(),
        request: Request {
            kind: Kind::Snapshot,
            folder: Uuid::nil(),
            name: "Thumbnail".into(),
            description: String::new(),
            permissions: Default::default(),
            expected_cost: 0,
            data,
        },
        thumbnail: Some(item),
    })
}
/// One paid native phase. Once submitted its outcome may be uncertain; there
/// is no retry and no private inventory fetch within this function.
pub async fn commit(http: &reqwest::Client, offer: Offer) -> Result<Committed, Error> {
    let thumbnail = offer.thumbnail;
    let request = offer.request;
    let response = http
        .post(offer.uploader)
        .header(
            "Content-Type",
            if thumbnail.is_some() {
                "application/jp2"
            } else {
                "application/octet-stream"
            },
        )
        .body(request.data)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|_| Error::Unknown)?;
    let ack = reply(response).await.map_err(|_| Error::Unknown)?;
    let item = thumbnail.unwrap_or_else(|| ack["new_inventory_item"].as_uuid());
    let asset = ack["new_asset"].as_uuid();
    if ack["state"].as_str() != "complete" || item.is_nil() || asset.is_nil() {
        return Err(Error::Unknown);
    }
    let (cost, cost_known) = match &ack["upload_price"] {
        Llsd::Integer(n) if *n >= 0 => (*n, true),
        _ => (request.expected_cost, false),
    };
    Ok(Committed {
        item,
        asset,
        folder: request.folder,
        kind: request.kind,
        cost,
        cost_known,
        approved_ceiling: request.expected_cost,
        thumbnail: thumbnail.is_some(),
    })
}
/// A separately authorized Read phase preserves actual server permissions.
pub async fn verify(http: &reqwest::Client, fetch: &str, owner: Uuid, committed: Committed) -> Result<Receipt, Error> {
    if committed.thumbnail {
        return Err(Error::Invalid);
    }
    let raw = reread(http, fetch, owner, committed.item).await?;
    let item = super::super::parse_items_response(&raw)
        .into_iter()
        .find(|item| {
            item.id == committed.item
                && item.asset_id == committed.asset
                && item.parent == committed.folder
                && item.asset_type == committed.kind.asset_type()
                && item.owner == owner
        })
        .ok_or(Error::Verification)?;
    Ok(Receipt {
        item,
        cost: committed.cost,
    })
}

/// The assistant's write/read releases are independent, one-shot and bounded.
pub async fn patch_thumbnail(http: &reqwest::Client, cap: &str, item: Uuid, folder: bool, asset: Uuid) -> Result<(), Error> {
    if item.is_nil() || asset.is_nil() {
        return Err(Error::Invalid);
    }
    let kind = if folder { "category" } else { "item" };
    let body = super::super::operations::metadata_thumbnail(asset);
    let mut response = http
        .patch(format!("{}/{kind}/{item}", cap.trim_end_matches('/')))
        .header("Content-Type", "application/llsd+xml")
        .body(aurora_llsd::to_xml(&body))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| Error::Unknown)?;
    if !response.status().is_success() {
        return Err(Error::Rejected);
    }
    let mut size = 0usize;
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unknown)? {
        size = size.checked_add(chunk.len()).ok_or(Error::Response)?;
        if size > 64 * 1024 {
            return Err(Error::Response);
        }
    }
    Ok(())
}
pub async fn verify_thumbnail(
    http: &reqwest::Client,
    cap: &str,
    owner: Uuid,
    item: Uuid,
    folder: bool,
    parent: Uuid,
    asset: Uuid,
) -> Result<ThumbnailReceipt, Error> {
    let kind = if folder { "category" } else { "item" };
    let response = http
        .get(format!("{}/{kind}/{item}", cap.trim_end_matches('/')))
        .header("Accept", "application/llsd+xml")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| Error::Unavailable)?;
    let doc = reply(response).await?;
    if folder {
        if doc["category_id"].as_uuid() != item
            || (doc.has("agent_id") && doc["agent_id"].as_uuid() != owner)
            || !doc.has("name")
            || !doc.has("version")
        {
            return Err(Error::Verification);
        }
        let f = super::super::folder_from_llsd(&doc)
            .filter(|f| f.id == item && f.parent == parent && f.thumbnail == asset)
            .ok_or(Error::Verification)?;
        Ok(ThumbnailReceipt::Folder(f))
    } else {
        let i = super::super::ais::parse_item_reply(&doc, item, owner).map_err(|_| Error::Verification)?;
        if i.owner != owner || i.parent != parent || i.thumbnail != asset {
            return Err(Error::Verification);
        }
        Ok(ThumbnailReceipt::Item(Box::new(i)))
    }
}
