from app.layers.models import (
    Layer,
    LayerCreate,
    LayerRead,
    LayerUpdate,
    LayerReadAuthenticated,
    LayerVariables,
    LayerGroupsRead,
    LayerUpdateBatch,
)
from app.db import get_session, AsyncSession
from fastapi import (
    Depends,
    HTTPException,
    APIRouter,
    Query,
    BackgroundTasks,
    Request,
)
from app.crud import CRUD
from app.layers.services import (
    get_count,
    get_all_authenticated,
    get_one,
    create_one,
    update_one,
    delete_one,
)
from typing import Any
from sqlmodel import select
import httpx
from app.config import config
from app.auth import require_admin, User
from app.geoserver.services import (
    update_local_db_with_layer_style,
    delete_coveragestore,
    delete_coveragestore_files,
)
from app.layers.uploads.views import router as uploads_router
from uuid import UUID
from fastapi.responses import Response
import aioboto3
from botocore.exceptions import NoCredentialsError, PartialCredentialsError
from fastapi.responses import JSONResponse
from rasterio.io import MemoryFile
import rasterio
from app.s3.services import get_s3
import base64

router = APIRouter()

router.include_router(uploads_router, prefix="/uploads", tags=["uploads"])


# TEMP_LAYERNAME: str = "barley-yield-google-blocksize-low"
TEMP_LAYERNAME: str = "BARLEY_Yield_cogeo"
# TEMP_LAYERNAME: str = "barley_yielf_cog_no_overviews"
# TEMP_LAYERNAME: str = "barley_pcr-globwb_hadgem2-es_historical_etb_2000"


@router.get("/tile/{z}/{x}/{y}.{image_format}")
async def get_tile(
    z: int,
    x: int,
    y: int,
    image_format: str,
    s3: aioboto3.Session = Depends(get_s3),
):
    """Get tile from S3 bucket and serve

    Get signed S3 URL and give to titiler to serve back to client
    """

    signed_url = await s3.generate_presigned_url(
        ClientMethod="get_object",
        Params={
            "Bucket": config.S3_BUCKET_ID,
            "Key": f"{config.S3_PREFIX}/{TEMP_LAYERNAME}.tif",
        },
        # ExpiresIn=10000,
    )
    print(signed_url)

    url, params = signed_url.split("?")
    encoded_params = base64.b64encode(params.encode())
    # signed_url
    # url_b64 = base64.b64encode(signed_url.encode())
    print(url)
    print(encoded_params)
    # print()
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"http://titiler:8000/cog/tiles/WebMercatorQuad/{z}/{x}/{y}.png",
            # f"http://titiler:8000/cog/info",
            params={
                "url": url,
                "url_params": encoded_params.decode(),
            },
        )

        print(response.status_code)
        print(response.content)

        return Response(content=response.content)


@router.post("/sync_styles", response_model=bool)
async def sync_style_from_geoserver_to_all_layers(
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
    user: User = Depends(require_admin),
) -> bool:
    """Syncs the style from geoserver to all layers"""

    # Get all layers
    query = select(
        Layer,
        Layer.id,
    )
    res = await session.exec(query)
    layers = res.all()

    for layer in layers:
        # Get the style from geoserver
        background_tasks.add_task(
            update_local_db_with_layer_style,
            layer_id=layer.id,
            session=session,
        )

    return True


@router.get("/groups", response_model=LayerGroupsRead)
async def get_groups(
    session: AsyncSession = Depends(get_session),
) -> LayerGroupsRead:
    """Get all unique groups
    This endpoint allows the menu to be populated with available keys
    """

    groups = {}
    for group in LayerVariables:
        # Get distinct values for each group

        column = getattr(Layer, group.value)
        res = await session.exec(select(column).distinct())
        groups[group.value] = [row for row in res.all()]

    return groups


@router.get("/map", response_model=list[LayerRead])
async def get_all_map_layers(
    session: AsyncSession = Depends(get_session),
    crop: str = Query(...),
    water_model: str | None = Query(...),
    climate_model: str | None = Query(...),
    scenario: str | None = Query(...),
    variable: str | None = Query(...),
    year: int | None = Query(...),
) -> list[LayerRead]:
    """Get all Layer data for the Drop4Crop map

    Does not include disabled layers (enabled=False)
    """

    query = select(Layer).where(Layer.enabled == True)

    if crop:
        query = query.where(Layer.crop == crop)
    if water_model:
        query = query.where(Layer.water_model == water_model)
    if climate_model:
        query = query.where(Layer.climate_model == climate_model)
    if scenario:
        query = query.where(Layer.scenario == scenario)
    if variable:
        query = query.where(Layer.variable == variable)
    if year:
        query = query.where(Layer.year == year)

    res = await session.exec(query)

    return res.all()


@router.get("/{layer_id}/value")
async def get_pixel_value(
    layer_id: str,
    lat: float,
    lon: float,
    s3: aioboto3.Session = Depends(get_s3),
):

    try:
        s3_response = await s3.get_object(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{TEMP_LAYERNAME}.tif",
        )
        content = await s3_response["Body"].read()

        with MemoryFile(content) as memfile:
            with memfile.open() as dataset:
                # Convert lat/lon to dataset coordinates
                row, col = dataset.index(lon, lat)
                rows, cols = rasterio.transform.rowcol(
                    dataset.transform, [lon], [lat]
                )
                # Get the pixel value at coordinates
                value = dataset.read(1)[row, col]

                return JSONResponse(content={"value": value})

    except Exception as e:
        print(e)
        raise HTTPException(status_code=500, detail=str(e))


@router.get("/cogs/{layer_id}")
async def get_layer_cog(
    # obj: LayerRead = Depends(get_one),
    # user: User = Depends(require_admin),
    request: Request,
    layer_id: str,
    session: AsyncSession = Depends(get_session),
    s3: aioboto3.Session = Depends(get_s3),
) -> LayerReadAuthenticated:
    """Get the cog of a layer by its UUID"""
    print(layer_id)
    print(request.headers)

    try:
        range_header = request.headers.get("range")
        print(f"Range header pre if: {range_header}")

        if range_header:
            print(f"Range header: {range_header}")
            range_value = range_header.strip().replace("bytes=", "")
            start, end = range_value.split("-")
            start = int(start) if start else None
            end = int(end) if end else None

            # Get the object from S3
            s3_response = await s3.get_object(
                Bucket=config.S3_BUCKET_ID,
                # Key=f"{S3_PREFIX}/{obj.layer_name}.tif",
                Key=f"{config.S3_PREFIX}/{layer_id}",
                Range=f"bytes={start}-{end}",
            )
            content = await s3_response["Body"].read()
            # print(f"S3 Resposne: {s3_response}")
            # Make a request to get total size of file to return in header
            # s3_response_total = await s3.head_object(
            #     Bucket=config.S3_BUCKET_ID,
            #     Key=f"{config.S3_PREFIX}/{TEMP_LAYERNAME}.tif",
            # )

            # total_size = s3_response_total["ContentLength"]
            # print(f"Total size: {total_size}")

            # Set the appropriate headers for COG tiling
            headers = {
                "Content-Type": "image/tiff",
                # 'Content-Disposition': f'inline; filename="{file_key}"',
                "Content-Disposition": f'inline; filename="{layer_id}"',
                "Access-Control-Allow-Origin": "*",
                "Access-Control-Allow-Headers": "Range",
                "Access-Control-Expose-Headers": "Content-Length,Content-Range,Accept-Ranges",
                "Accept-Ranges": "bytes",
                "Content-Length": str(s3_response["ContentLength"]),
                "Content-Range": str(s3_response["ContentRange"]),
            }
            return Response(content, status_code=206, headers=headers)
        else:
            # We don't really want to return the whole file, so we return a 206
            # with a small chunk of the file
            print("NO HEADER PROVIDED!")
            s3_response = await s3.get_object(
                Bucket=config.S3_BUCKET_ID,
                # Key=f"{S3_PREFIX}/{obj.layer_name}.tif",
                Key=f"{config.S3_PREFIX}/{layer_id}",
                # Range="bytes=0-1000",
            )
            content = await s3_response["Body"].read()

            headers = {
                "Content-Type": "image/tiff",
                # "Content-Disposition": f'inline; filename="{obj.layer_name}.tif"',
                "Content-Disposition": f'inline; filename="{layer_id}"',
                "Access-Control-Allow-Origin": "*",
                "Access-Control-Allow-Headers": "Range",
                "Access-Control-Expose-Headers": "Content-Length,Content-Range,Accept-Ranges",
                "Accept-Ranges": "bytes",
                "Content-Length": str(s3_response["ContentLength"]),
            }

            return Response(content, headers=headers)

    except s3.exceptions.NoSuchKey:
        raise HTTPException(status_code=404, detail="File not found")
    except NoCredentialsError:
        raise HTTPException(
            status_code=500, detail="Credentials not available"
        )
    except PartialCredentialsError:
        raise HTTPException(status_code=500, detail="Incomplete credentials")
    except Exception as e:
        print(e)
        # Print also the function name
        import traceback

        print(traceback.format_exc())

        raise HTTPException(status_code=500, detail=str(e))


@router.head("/cogs/{layer_id}")
async def get_layer_cog_head(
    # obj: LayerRead = Depends(get_one),
    # user: User = Depends(require_admin),
    layer_id: str,
    session: AsyncSession = Depends(get_session),
    s3: aioboto3.Session = Depends(get_s3),
) -> LayerReadAuthenticated:
    """Get the head of a layer by its UUID"""

    try:
        s3_response = await s3.head_object(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{layer_id}",
        )
        headers = {
            "Content-Type": "image/tiff",
            "Content-Disposition": f'inline; filename="{layer_id}"',
            "Access-Control-Allow-Origin": "*",
            "Access-Control-Allow-Headers": "Range",
            "Access-Control-Expose-Headers": "Content-Length,Content-Range,Accept-Ranges",
            "Accept-Ranges": "bytes",
            "Content-Length": str(s3_response["ContentLength"]),
        }

        return Response(headers=headers)

    except Exception as e:
        print(e)
        raise HTTPException(status_code=500, detail=str(e))


@router.get("/{layer_id}", response_model=LayerReadAuthenticated)
async def get_layer(
    obj: LayerRead = Depends(get_one),
    user: User = Depends(require_admin),
    session: AsyncSession = Depends(get_session),
) -> LayerReadAuthenticated:
    """Get a layer by id"""

    res = LayerReadAuthenticated.model_validate(obj)

    # Get layer information from geoserver
    async with httpx.AsyncClient() as client:
        # Get layer styling information from geoserver
        response = await client.get(
            f"{config.GEOSERVER_URL}/rest/layers/{config.GEOSERVER_WORKSPACE}"
            f":{res.layer_name}.json",
            auth=(config.GEOSERVER_USER, config.GEOSERVER_PASSWORD),
        )
        if response.status_code == 200:
            res.style_name = response.json()["layer"]["defaultStyle"]["name"]
            res.created_at = response.json()["layer"]["dateCreated"]

    if res.style_name != obj.style_name:
        # Update the style in the database
        update_local_db_with_layer_style(layer_id=res.id, session=session)

    return obj


@router.get("", response_model=list[LayerReadAuthenticated])
async def get_all_layers(
    data: Any = Depends(get_all_authenticated),
    session: AsyncSession = Depends(get_session),
    total_count: int = Depends(get_count),
    user: User = Depends(require_admin),
) -> list[LayerReadAuthenticated]:
    """Get all Layer data

    If token and user is admin, return additional fields
    """

    return data


@router.post("", response_model=LayerReadAuthenticated)
async def create_layer(
    create_obj: LayerCreate,
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
    user: User = Depends(require_admin),
) -> LayerReadAuthenticated:
    """Creates a layer data record"""

    obj = await create_one(create_obj.model_dump(), session, background_tasks)

    return obj


@router.put("/batch", response_model=list[LayerReadAuthenticated])
async def update_many(
    layer_batch: LayerUpdateBatch,
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
    user: User = Depends(require_admin),
) -> list[LayerReadAuthenticated]:
    """Update plots from a list of PlotUpdate objects"""

    objs = []
    for id in layer_batch.ids:
        update_obj = LayerUpdate.model_validate(layer_batch.data)
        obj = await update_one(
            layer_id=id,
            layer_update=update_obj,
            session=session,
            background_tasks=background_tasks,
        )
        objs.append(obj)

    return objs


@router.put("/{layer_id}", response_model=LayerReadAuthenticated)
async def update_layer(
    updated_layer: Layer = Depends(update_one),
    user: User = Depends(require_admin),
) -> LayerReadAuthenticated:
    """Update a layer by id"""

    return updated_layer


@router.delete("/batch", response_model=list[UUID])
async def delete_batch(
    ids: list[UUID],
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
) -> list[UUID]:
    """Delete by a list of ids"""

    deleted_ids = []
    for id in ids:
        # Delete the layer from geoserver
        background_tasks.add_task(delete_one, id, session)
        deleted_ids.append(id)

    return deleted_ids


@router.delete("/{layer_id}")
async def delete_layer(
    layer: UUID = Depends(delete_one),
    user: User = Depends(require_admin),
) -> Any:
    """Delete a layer by id"""

    return layer
