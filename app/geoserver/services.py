from typing import List
import httpx
from app.config import config
from app.layers.models import Layer
from app.db import AsyncSession
from tenacity import retry, stop_after_attempt, wait_fixed
from uuid import UUID
from sqlmodel import select
from osgeo import gdal, gdalconst
import os


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


async def update_local_db_with_layer_style(
    layer_id: UUID,
    session: AsyncSession,
) -> None:
    """Update the layer style in the database from geoserver

    As we keep the style name in the database, we need a way to fix it if
    something pushes it out of sync
    """

    res = await session.exec(select(Layer).where(Layer.id == layer_id))
    obj = res.one_or_none()

    # Get style name from geoserver
    obj.style_name = await get_layer_style(layer_name=obj.layer_name)

    session.add(obj)
    await session.commit()


def convert_to_cog_in_memory(input_bytes: bytes) -> bytes:
    """Convert in-memory GeoTIFF to Cloud Optimized GeoTIFF in memory using GDAL"""

    print("Converting to COG")
    # Create an in-memory file from the input bytes
    input_filename = "/vsimem/input.tif"
    gdal.FileFromMemBuffer(input_filename, input_bytes)

    # Output in-memory file for the COG
    output_filename = "/vsimem/output-cog.tif"
    options = gdal.TranslateOptions(
        format="COG", creationOptions=["OVERVIEWS=NONE"]
    )
    gdal.Translate(output_filename, input_filename, options=options)

    # Read the in-memory COG file back to a byte array
    output_ds = gdal.VSIFOpenL(output_filename, "rb")
    gdal.VSIFSeekL(output_ds, 0, os.SEEK_END)
    size = gdal.VSIFTellL(output_ds)
    gdal.VSIFSeekL(output_ds, 0, os.SEEK_SET)
    cog_bytes = gdal.VSIFReadL(1, size, output_ds)
    gdal.VSIFCloseL(output_ds)

    # Clean up in-memory files
    gdal.Unlink(input_filename)
    gdal.Unlink(output_filename)
    print("COG conversion successful")

    return cog_bytes


async def upload_bytes(data: bytes):
    """Async bytes generator to yield byte content to AsyncClient"""
    yield data


async def upload_cog_to_geoserver(cog_bytes: bytes, store_name: str):
    """Upload COG to GeoServer"""
    url = f"{config.GEOSERVER_URL}/rest/workspaces/{config.GEOSERVER_WORKSPACE}/coveragestores/{store_name}/file.geotiff"
    headers = {"Content-type": "image/tiff"}

    async with httpx.AsyncClient() as client:
        response = await client.put(
            url,
            headers=headers,
            content=upload_bytes(cog_bytes),
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
        print("STATUS CODE", response.status_code)
        if response.status_code == 201:
            print(f"Successfully uploaded and published COG for {store_name}")
        else:
            print(
                f"Failed to upload and publish COG for {store_name}: {response.content}"
            )
            response.raise_for_status()


async def process_and_upload_geotiff(input_bytes: bytes, store_name: str):
    """Process in-memory GeoTIFF to COG and upload to GeoServer"""
    try:
        cog_bytes = convert_to_cog_in_memory(input_bytes)
        await upload_cog_to_geoserver(cog_bytes, store_name)
    except Exception as e:
        print(f"Exception occurred for {store_name}: {e}")
