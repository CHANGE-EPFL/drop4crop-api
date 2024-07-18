from typing import List
import httpx
from app.config import config
from app.layers.models import Layer
from app.db import AsyncSession
from tenacity import retry, stop_after_attempt, wait_fixed
from uuid import UUID
from sqlmodel import select


async def get_layer_style(layer_name: str) -> str:
    """Get layer style from geoserver"""

    async with httpx.AsyncClient() as client:
        # Get layer styling information from geoserver
        response = await client.get(
            f"{config.GEOSERVER_URL}/rest/layers/"
            f"{config.GEOSERVER_WORKSPACE}"
            f":{layer_name}.json",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
        if response.status_code == 200:
            return response.json()["layer"]["defaultStyle"]["name"]


@retry(stop=stop_after_attempt(25), wait=wait_fixed(2))
async def update_style_in_geoserver(
    layer_id: UUID,
    session: AsyncSession,
    layer_name: str,
    style_name: str,
):
    async with httpx.AsyncClient() as client:
        # Check if the style exists
        response = await client.get(
            f"{config.GEOSERVER_URL}/rest/styles/{style_name}.json",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
        response.raise_for_status()

        # Update the style in geoserver
        response = await client.put(
            f"{config.GEOSERVER_URL}/rest/layers/"
            f"{config.GEOSERVER_WORKSPACE}:{layer_name}",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
            json={
                "layer": {
                    "defaultStyle": {
                        "name": style_name,
                    }
                }
            },
        )
        response.raise_for_status()

        # Get the style from geoserver
        updated_layer = await get_layer_style(layer_name=layer_name)

        # Update the style in the database
        res = await session.exec(select(Layer).where(Layer.id == layer_id))
        obj = res.one_or_none()
        obj.style_name = updated_layer
        session.add(obj)
        await session.commit()
