from fastapi import Query, APIRouter, Response, HTTPException, Depends
from typing import Any
import httpx
import json
from app.config import config
from app.styles.models import StyleCreate
import re
from app.auth import require_admin, User

router = APIRouter()


@router.get("/{style_id}")
async def get_style(
    style_id: str,
    user: User = Depends(require_admin),
):
    """Get a style by id

    Geoserver has a style by name, so we use the name as the id
    """

    async with httpx.AsyncClient() as client:
        # Get all styles then get href for the style, then get the style
        res = await client.get(
            f"{config.GEOSERVER_URL}/rest/styles/{style_id}.json",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
        if res.status_code == 200:
            style = res.json()["style"]
        else:
            style = {}

    style["id"] = style["name"]

    # Get style information from geoserver by requesting the same resource but
    # with a content type of application/vnd.ogc.sld+xml
    async with httpx.AsyncClient() as client:
        res = await client.get(
            f"{config.GEOSERVER_URL}/rest/styles/{style_id}.sld",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
            headers={"Content-Type": "application/vnd.ogc.sld+xml"},
        )
        if res.status_code == 200:
            style["sld"] = res.text
        else:
            style["sld"] = ""
    return style


@router.get("", response_model=list[Any])
async def get_all_styles(
    response: Response,
    filter: str = Query(None),
    range: str = Query(None),
    sort: str = Query(None),
    user: User = Depends(require_admin),
) -> list[Any]:
    """Get all styles from geoserver"""

    range = json.loads(range) if range else []
    filter = json.loads(filter) if filter else {}
    sort = json.loads(sort) if sort else {}

    # Get styles from geoserver
    async with httpx.AsyncClient() as client:
        res = await client.get(
            f"{config.GEOSERVER_URL}/rest/styles",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
    if res.status_code == 200:
        styles = res.json()["styles"]["style"]
    else:
        styles = []

    for style in styles:
        style["id"] = style["name"]

    # # Sort
    # if sort:
    #     styles = sorted(styles, key=lambda x: x[sort["field"]])

    # # Filter
    # if filter:
    #     for key, value in filter.items():
    #         styles = [style for style in styles if style[key] == value]
    total_count = len(styles)

    if len(range) == 2:
        start, end = range
    else:
        start, end = [0, total_count]  # For content-range header

    response.headers["Content-Range"] = f"styles {start}-{end}/{total_count}"

    return styles


@router.post("")
async def create_style(
    style: StyleCreate,
    user: User = Depends(require_admin),
):
    """Create a style on geoserver"""

    # Check to see if the style already exists
    async with httpx.AsyncClient() as client:
        res = await client.get(
            f"{config.GEOSERVER_URL}/rest/styles/{style.name}.json",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
    if res.status_code == 200:
        raise HTTPException(
            status_code=409,
            detail="Style already exists",
        )
    # Fix the style name in the provided XML (sld field) to match style name
    # provided in the style object
    style_name = style.name

    # Find the value in between <name> OR <sld:name> tags (case insensitive)
    # and replace it with the style name
    style.sld = re.sub(
        r"<name>.*</name>|<sld:name>.*</sld:name>",
        f"<name>{style_name}</name>",
        style.sld,
        flags=re.IGNORECASE,
    )

    # Upload the XML style directly to geoserver
    async with httpx.AsyncClient() as client:
        res = await client.post(
            f"{config.GEOSERVER_URL}/rest/styles",
            headers={"Content-Type": "application/vnd.ogc.sld+xml"},
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
            data=style.sld,
        )

        if res.status_code > 299:
            raise HTTPException(
                status_code=res.status_code,
                detail=res.text,
            )

    # Get the style from geoserver
    async with httpx.AsyncClient() as client:
        res = await client.get(
            f"{config.GEOSERVER_URL}/rest/styles/{style_name}.json",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
        if res.status_code:
            style_res = res.json()["style"]
            style_res["id"] = style_res["name"]
        else:
            style_res = {}

    if res.status_code > 299:
        raise HTTPException(
            status_code=res.status_code,
            detail=res.text,
        )

    return style_res


@router.put("/{style_id}")
async def update_style(
    style_id: str,
    style: StyleCreate,
    user: User = Depends(require_admin),
):
    """Update a style on geoserver"""

    # Check to see if the style already exists
    async with httpx.AsyncClient() as client:
        res = await client.get(
            f"{config.GEOSERVER_URL}/rest/styles/{style_id}.json",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
    if res.status_code > 299:
        raise HTTPException(
            status_code=404,
            detail="Style not found",
        )

    # Fix the style name in the provided XML (sld field) to match style name
    # provided in the style object
    style_name = style_id

    # Find the value in between <name> OR <sld:name> tags (case insensitive)
    # and replace it with the style name
    style.sld = re.sub(
        r"<name>.*</name>|<sld:name>.*</sld:name>",
        f"<name>{style_name}</name>",
        style.sld,
        flags=re.IGNORECASE,
    )

    # Upload the XML style directly to geoserver
    async with httpx.AsyncClient() as client:
        res = await client.put(
            f"{config.GEOSERVER_URL}/rest/styles/{style_id}",
            headers={"Content-Type": "application/vnd.ogc.sld+xml"},
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
            data=style.sld,
        )

        if res.status_code > 299:
            raise HTTPException(
                status_code=res.status_code,
                detail=res.text,
            )

    # Get the style from geoserver
    async with httpx.AsyncClient() as client:
        res = await client.get(
            f"{config.GEOSERVER_URL}/rest/styles/{style_name}.json",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
        if res.status_code:
            style_res = res.json()["style"]
            style_res["id"] = style_res["name"]
        else:
            style_res = {}

    if res.status_code > 299:
        raise HTTPException(
            status_code=res.status_code,
            detail=res.text,
        )

    return style_res


@router.delete("/{style_id}")
async def delete_style(
    style_id: str,
    user: User = Depends(require_admin),
) -> None:
    """Delete a style"""

    async with httpx.AsyncClient() as client:
        res = await client.delete(
            f"{config.GEOSERVER_URL}/rest/styles/{style_id}",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )

    if res.status_code != 200:
        raise HTTPException(
            status_code=res.status_code,
            detail=res.text,
        )
    return style_id
