import logging
from fastapi import FastAPI, Query, Depends, APIRouter, Response, HTTPException
from sqlalchemy import text
from sqlalchemy.ext.asyncio import AsyncSession
from starlette.responses import StreamingResponse
from app.db import get_session
from io import BytesIO
import typing

# Initialize FastAPI app
app = FastAPI()

router = APIRouter()

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


# Service method for retrieving WMS data from database
async def get_wms(
    db: AsyncSession,
    width: int,
    height: int,
    bbox: str,
    crs: int,
    format: str = "image/png",
) -> typing.BinaryIO:
    async with db.begin():
        params = {
            "format": format,
            "width": width,
            "height": height,
            "crs": crs,
            "bbox": bbox,
            "table": "observations",
        }

        # Log the parameters
        logger.info(f"Parameters: {params}")

        # Required for ST_AsPNG and ST_AsJPEG to work
        await db.execute(
            text("SET postgis.gdal_enabled_drivers = 'ENABLE_ALL';"),
        )
        await db.execute(
            text("SET postgis.enable_outdb_rasters TO true;"),
        )

        result = await db.execute(
            text(
                "SELECT get_rast_tile(:format, :width, :height, :crs, :bbox, 'public', :table)"
            ),
            params,
        )
        # print(result.context.statement)

        result = result.first()

        # Print query in raw SQL

        # Check if result is None
        if result is None:
            logger.error("No result returned from the database query.")
            raise ValueError(
                "No raster data found for the specified parameters."
            )

        return BytesIO(result[0])


# Make a WMS compatible getcapabilites endpoint
#  /wms?SERVICE=WMS&REQUEST=GetCapabilities


@router.get("")
async def wms(
    layers: str = Query(
        # None,
        # regex=r"^observations",
        # alias="LAYERS",
    ),
    format: str = Query(
        "image/png",
        # regex="^image/(png|jpeg)$",
        # alias="FORMAT",
    ),
    width: int = Query(
        None,
        gt=0,
        # alias="WIDTH",
    ),
    height: int = Query(
        None,
        gt=0,
        # alias="HEIGHT",
    ),
    bbox: str = Query(
        None,
        # regex=r"^[\d\.-]+,[\d\.-]+,[\d\.-]+,[\d\.-]+$",
        # alias="BBOX",
    ),
    crs: str = Query(
        None,
        regex=r"^EPSG:\d+$",
        # alias="CRS",
    ),
    *,
    service: str = Query(
        None,
        regex=r"^WMS$",
        alias="SERVICE",
    ),
    request: str = Query(
        "GetMap", regex=r"^(GetCapabilities|GetMap)$", alias="REQUEST"
    ),
    db: AsyncSession = Depends(get_session),
):

    if not all([layers, format, width, height, bbox, crs]):
        raise HTTPException(
            status_code=400,
            detail="Missing required parameters for GetMap request."
            f"Got: {layers}, {format}, {width}, {height}, {bbox}, {crs}"
            f"Expected: layers, format, width, height, bbox, crs",
        )

    # raise HTTPException(404)  # Mock a 404 for testing
    # Call the service method to get the raster data
    raster_data = await get_wms(
        db=db,
        width=width,
        height=height,
        bbox=bbox,
        crs=int(crs.split(":")[1]),
        format=format,
    )
    # Return the raster data as a streaming response
    return StreamingResponse(
        content=raster_data,
        media_type=format,
    )
