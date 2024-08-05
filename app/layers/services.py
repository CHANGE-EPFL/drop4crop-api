from app.layers.models import (
    Layer,
    LayerRead,
    LayerReadAuthenticated,
    LayerCreate,
    LayerUpdate,
)
from app.db import get_session, AsyncSession
from fastapi import Depends, APIRouter, Query, Response, HTTPException
from uuid import UUID
from app.crud import CRUD
from app.config import config
from app.s3.services import get_s3
import aioboto3

router = APIRouter()
crud = CRUD(Layer, LayerRead, LayerCreate, LayerUpdate)


async def get_count(
    response: Response,
    filter: str = Query(None),
    range: str = Query(None),
    sort: str = Query(None),
    session: AsyncSession = Depends(get_session),
):
    count = await crud.get_total_count(
        response=response,
        sort=sort,
        range=range,
        filter=filter,
        session=session,
    )

    return count


async def get_all_authenticated(
    filter: str = Query(None),
    sort: str = Query(None),
    range: str = Query(None),
    session: AsyncSession = Depends(get_session),
):

    res = await crud.get_model_data(
        sort=sort,
        range=range,
        filter=filter,
        session=session,
    )

    read_layers = []
    for layer in res:
        obj = LayerReadAuthenticated.model_validate(layer)

        read_layers.append(obj)

    return read_layers


async def get_one(
    layer_id: UUID,
    session: AsyncSession = Depends(get_session),
) -> LayerRead:
    res = await crud.get_model_by_id(model_id=layer_id, session=session)

    if not res:
        raise HTTPException(
            status_code=404, detail=f"ID: {layer_id} not found"
        )
    return res


async def create_one(
    data: LayerCreate,
    session: AsyncSession = Depends(get_session),
) -> Layer:
    """Create a single layer

    To be used in both create one and create many endpoints
    """

    obj = Layer.model_validate(data)

    session.add(obj)

    await session.commit()
    await session.refresh(obj)

    return obj


async def update_one(
    layer_id: UUID,
    layer_update: LayerUpdate,
    session: AsyncSession = Depends(get_session),
) -> Layer:
    """Update a single layer"""

    obj = await crud.get_model_by_id(model_id=layer_id, session=session)

    update_data = layer_update.model_dump(exclude_unset=True)

    obj.sqlmodel_update(update_data)

    session.add(obj)
    await session.commit()
    await session.refresh(obj)

    obj = await get_one(layer_id, session)

    return obj


async def delete_one(
    layer_id: UUID,
    session: AsyncSession = Depends(get_session),
    s3: aioboto3.Session = Depends(get_s3),
) -> UUID:
    """Delete a single layer"""
    obj = await crud.get_model_by_id(model_id=layer_id, session=session)

    if not obj:
        raise HTTPException(
            status_code=404, detail=f"ID: {layer_id} not found"
        )

    try:
        # Delete from S3 using obj.filename plus S3 prefix
        await s3.delete_object(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{obj.filename}",
        )
    except Exception as e:
        raise HTTPException(
            status_code=400,
            detail={
                "message": "Error deleting file from S3. ",
                "error": str(e),
            },
        )

    await session.delete(obj)
    await session.commit()

    return layer_id
